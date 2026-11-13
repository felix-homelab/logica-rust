//! The MCP server handler: tools, resources, subscriptions and prompts over the shared engine.

use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, GetPromptRequestParams,
    GetPromptResponse, GetPromptResult, Implementation, ListPromptsResult, ListResourceTemplatesResult,
    ListResourcesResult, ListToolsResult, PaginatedRequestParams, ProgressNotificationParam, Prompt,
    PromptArgument, PromptMessage, ReadResourceRequestParams, ReadResourceResponse, ReadResourceResult,
    Resource, ResourceContents, ResourceTemplate, ResourceUpdatedNotificationParam, Role, ServerCapabilities,
    ServerConfig, SubscribeRequestParams, SubscriptionFilter, Tool, ToolAnnotations,
};
use rmcp::service::{RequestContext, SubscriptionContext};
use rmcp::{ErrorData, RoleServer, ServerHandler};
use serde_json::{Map, Value, json};
use tokio::sync::broadcast::error::RecvError;

use logica_engine::{Actor, CommandError, ErrorCode, Event};

use crate::resources::{self, Target};
use crate::shared::Shared;
use crate::text::{self, Captures};
use crate::tools::{self, Kind, ToolDef};
use crate::{prompts, render_note};

const INSTRUCTIONS: &str = "Logica is a logic analyser shared with a person who may be watching in its window. \
Start with the session tool. Open recordings with open_file or capture from a device, find out what is on the wire with summarize, \
decode with decoder, read results with get_decoded, and point the person at findings with show_user. \
Times are given in seconds with the sample index; pass sample indices back for exact follow-up. \
Everything an agent changes can be undone (run_command history.undo).";

/// One MCP client's view of the shared engine. Cheap to clone; one per connection or request.
#[derive(Clone)]
pub struct Logica {
    shared: Arc<Shared>,
    tools: Arc<Vec<Tool>>,
    contract: Arc<Value>,
    /// URIs subscribed with the legacy `resources/subscribe` on this connection.
    legacy_subs: Arc<Mutex<Vec<String>>>,
    legacy_task: Arc<OnceLock<()>>,
}

/// A tool's result before it becomes MCP content.
struct Outcome {
    text: String,
    structured: Value,
    image: Option<String>,
}

fn failed(message: impl Into<String>) -> CommandError {
    CommandError::new(ErrorCode::Failed, message)
}

fn invalid(message: impl Into<String>) -> CommandError {
    CommandError::new(ErrorCode::InvalidInput, message)
}

impl Logica {
    pub fn new(shared: Arc<Shared>) -> Self {
        let contract = logica_engine::schema::contract();
        let tools = tools::TOOLS.iter().map(|t| tool_of(t, &contract)).collect();
        Logica {
            shared,
            tools: Arc::new(tools),
            contract: Arc::new(contract),
            legacy_subs: Arc::default(),
            legacy_task: Arc::default(),
        }
    }

    /// Every command an agent sends carries `agent:<its client name>` (doc 03).
    fn actor(ctx: &RequestContext<RoleServer>) -> Actor {
        let name = ctx.client_info().map(|i| i.name).unwrap_or_default();
        let name = name.trim();
        Actor::Agent(if name.is_empty() { "agent".into() } else { name.into() })
    }

    async fn run(&self, actor: &Actor, command: &str, input: Value) -> Result<Value, CommandError> {
        self.shared.run_async(actor.clone(), command.into(), input).await
    }

    async fn captures(&self, actor: &Actor) -> Captures {
        match self.run(actor, "session.get", json!({})).await {
            Ok(s) => Captures::from_session(&s),
            Err(_) => Captures::default(),
        }
    }

    async fn call(
        &self,
        tool: &ToolDef,
        mut args: Map<String, Value>,
        ctx: &RequestContext<RoleServer>,
    ) -> Result<Outcome, CommandError> {
        let actor = Self::actor(ctx);
        let capture = args.get("capture").and_then(Value::as_str).unwrap_or("").to_string();
        let (command, action) = match &tool.kind {
            Kind::Command(c) => (*c, String::new()),
            Kind::Actions(list) => {
                let action =
                    args.remove("action").and_then(|a| a.as_str().map(String::from)).unwrap_or_default();
                let Some((_, c)) = list.iter().find(|(a, _)| *a == action) else {
                    let names: Vec<&str> = list.iter().map(|(a, _)| *a).collect();
                    return Err(invalid(format!("{} needs an action: {}.", tool.name, names.join(", "))));
                };
                (*c, action)
            }
            Kind::Capture => return self.capture(args, &actor, ctx).await,
            Kind::RunCommand => {
                let Some(command) = args.get("command").and_then(Value::as_str).map(String::from) else {
                    return Err(invalid(
                        "run_command needs the command's name; the list is in logica://commands.",
                    ));
                };
                let input = args.remove("input").unwrap_or_else(|| json!({}));
                let out = self.run(&actor, &command, input).await?;
                return Ok(Outcome { text: text::generic(&command, &out), structured: out, image: None });
            }
        };
        let mut out = self.run(&actor, command, Value::Object(args)).await?;
        let caps = || self.captures(&actor);
        let text = match tool.name {
            "session" => text::session(&out),
            "list_devices" => text::devices(&out),
            "device" if action == "connect" => {
                format!("Connected; the lease is yours.\n{}", text::generic(command, &out))
            }
            "open_file" => format!("Opened {}.", text::capture_line(&out["capture"])),
            "save_capture" => {
                format!("Saved to {} ({} bytes).", out["path"].as_str().unwrap_or(""), out["bytes"])
            }
            "summarize" => text::summary(&caps().await, &capture, &out),
            "measure" => text::measure(&caps().await, &capture, &action, &out),
            "decoder" => text::decoder(&caps().await, &capture, &action, &out),
            "get_decoded" => text::decoded(&caps().await, &capture, &out),
            "search" => text::search(&caps().await, &capture, &out),
            "show_user" => text::marker(&caps().await, &capture, &out),
            "render" => {
                let image = out
                    .as_object_mut()
                    .and_then(|m| m.remove("pngBase64"))
                    .and_then(|v| v.as_str().map(String::from));
                let text = render_note(&out);
                return Ok(Outcome { text, structured: out, image });
            }
            _ => text::generic(command, &out),
        };
        Ok(Outcome { text, structured: out, image: None })
    }

    /// `capture`: configure, start without blocking the engine, then wait for `capture.done` with
    /// progress notifications; a cancelled call stops the capture.
    async fn capture(
        &self,
        mut args: Map<String, Value>,
        actor: &Actor,
        ctx: &RequestContext<RoleServer>,
    ) -> Result<Outcome, CommandError> {
        let action = args.remove("action").and_then(|a| a.as_str().map(String::from)).unwrap_or_default();
        let device = args.get("device").and_then(Value::as_str).unwrap_or("").to_string();
        match action.as_str() {
            "stop" => {
                let out = self.run(actor, "capture.stop", json!({ "device": device })).await?;
                return Ok(Outcome {
                    text: format!("Stopped the capture on {device}; what was recorded is kept."),
                    structured: out,
                    image: None,
                });
            }
            "start" => {}
            _ => return Err(invalid("capture needs an action: start or stop.")),
        }
        let config = args.remove("config");
        let wait = args.remove("wait").and_then(|w| w.as_bool()).unwrap_or(true);
        let expected = config.as_ref().and_then(|c| {
            let (n, rate) = (c["samples"].as_f64()?, c["sampleRateHz"].as_f64()?);
            (rate > 0.0).then_some(n / rate)
        });
        if let Some(config) = config {
            self.run(actor, "device.configure", json!({ "device": device, "config": config })).await?;
        }
        let mut events = self.shared.events();
        args.insert("wait".into(), false.into());
        let started = self.run(actor, "capture.start", Value::Object(args)).await?;
        let id = started["id"].as_str().unwrap_or("").to_string();
        if !wait {
            return Ok(Outcome {
                text: format!(
                    "Capture {id} is running on {device}. The session shows it when it is done; stop it with capture (action stop)."
                ),
                structured: started,
                image: None,
            });
        }
        let token = ctx.meta.get_progress_token();
        let began = Instant::now();
        let mut cancelled = false;
        loop {
            tokio::select! {
                () = ctx.ct.cancelled(), if !cancelled => {
                    cancelled = true;
                    let _ = self.run(actor, "capture.stop", json!({ "device": device })).await;
                }
                e = events.recv() => match e {
                    Ok(Event::CaptureDone { capture, .. }) if capture == id => break,
                    Ok(_) | Err(RecvError::Lagged(_)) => {}
                    Err(RecvError::Closed) => return Err(failed("The engine stopped during the capture.")),
                },
                () = tokio::time::sleep(Duration::from_millis(50)) => {
                    let shared = self.shared.clone();
                    let _ = tokio::task::spawn_blocking(move || shared.engine().poll()).await;
                    if let (Some(token), Some(total)) = (token.clone(), expected) {
                        let fraction = (began.elapsed().as_secs_f64() / total).min(0.99);
                        let note = ProgressNotificationParam::new(token, (fraction * 100.0).round())
                            .with_total(100.0)
                            .with_message("estimated from the elapsed time");
                        let _ = ctx.peer.notify_progress(note).await;
                    }
                }
            }
        }
        let session = self.run(actor, "session.get", json!({})).await?;
        let summary =
            session["captures"].as_array().and_then(|a| a.iter().find(|c| c["id"] == id.as_str())).cloned();
        let line = summary.as_ref().map(text::capture_line).unwrap_or_else(|| id.clone());
        let text = if cancelled {
            format!("The capture was cancelled and stopped; what was recorded is kept: {line}.")
        } else {
            format!("Captured {line}. Next: summarize it.")
        };
        Ok(Outcome {
            text,
            structured: json!({ "id": id, "capture": summary, "cancelled": cancelled }),
            image: None,
        })
    }

    async fn read(&self, actor: &Actor, uri: &str) -> Result<Value, ErrorData> {
        let not_found = || ErrorData::resource_not_found(format!("There is no resource {uri}."), None);
        let target = resources::parse(uri).ok_or_else(not_found)?;
        let Some((command, input)) = resources::command(&target) else {
            return Ok((*self.contract).clone());
        };
        let out = self.run(actor, command, input).await.map_err(|e| match e.code {
            ErrorCode::NotFound => ErrorData::resource_not_found(e.message, None),
            _ => ErrorData::invalid_params(e.message, None),
        })?;
        match target {
            Target::Capture(id) => out["captures"]
                .as_array()
                .and_then(|a| a.iter().find(|c| c["id"] == id.as_str()))
                .cloned()
                .ok_or_else(|| {
                    ErrorData::resource_not_found(format!("No capture with the id \"{id}\" is open."), None)
                }),
            _ => Ok(out),
        }
    }

    /// Sends `resources/updated` for this connection's legacy subscriptions as events arrive.
    fn start_legacy_updates(&self, peer: rmcp::Peer<RoleServer>) {
        if self.legacy_task.set(()).is_err() {
            return;
        }
        let subs = self.legacy_subs.clone();
        let mut events = self.shared.events();
        tokio::spawn(async move {
            loop {
                let e = match events.recv().await {
                    Ok(e) => e,
                    Err(RecvError::Lagged(_)) => continue,
                    Err(RecvError::Closed) => return,
                };
                let subscribed = subs.lock().unwrap_or_else(std::sync::PoisonError::into_inner).clone();
                for changed in resources::changed_by(&e) {
                    for s in subscribed.iter().filter(|s| resources::matches(s, &changed)) {
                        if peer
                            .notify_resource_updated(ResourceUpdatedNotificationParam::new(s.clone()))
                            .await
                            .is_err()
                        {
                            return;
                        }
                    }
                }
            }
        });
    }
}

fn tool_of(t: &ToolDef, contract: &Value) -> Tool {
    let schema = tools::input_schema(t, contract);
    let annotations = ToolAnnotations::new().read_only(t.read_only).destructive(false).open_world(false);
    Tool::new(t.name, t.description, Arc::new(schema)).with_title(t.title).annotate(annotations)
}

fn tool_result(r: Result<Outcome, CommandError>) -> CallToolResult {
    match r {
        Ok(o) => {
            let mut content = vec![ContentBlock::text(o.text)];
            if let Some(png) = o.image {
                content.push(ContentBlock::image(png, "image/png"));
            }
            let mut result = CallToolResult::success(content);
            // structured content must be an object
            result.structured_content = Some(match o.structured {
                v @ Value::Object(_) => v,
                v => json!({ "result": v }),
            });
            result
        }
        Err(e) => {
            let mut result = CallToolResult::error(vec![ContentBlock::text(e.message.clone())]);
            result.structured_content = Some(json!({ "code": e.code, "message": e.message }));
            result
        }
    }
}

impl ServerHandler for Logica {
    fn get_info(&self) -> ServerConfig {
        let caps = ServerCapabilities::builder()
            .enable_tools()
            .enable_resources()
            .enable_resources_subscribe()
            .enable_prompts()
            .build();
        ServerConfig::new(caps)
            .with_server_info(Implementation::new("logica", env!("CARGO_PKG_VERSION")))
            .with_instructions(INSTRUCTIONS)
    }

    async fn list_tools(
        &self,
        _: Option<PaginatedRequestParams>,
        _: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        Ok(ListToolsResult::with_all_items((*self.tools).clone()))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        let Some(tool) = tools::find(&request.name) else {
            return Err(ErrorData::invalid_params(format!("There is no tool \"{}\".", request.name), None));
        };
        let args = request.arguments.unwrap_or_default();
        Ok(tool_result(self.call(tool, args, &ctx).await).into())
    }

    async fn list_resources(
        &self,
        _: Option<PaginatedRequestParams>,
        _: RequestContext<RoleServer>,
    ) -> Result<ListResourcesResult, ErrorData> {
        let list = resources::FIXED
            .iter()
            .map(|(uri, name, desc)| {
                Resource::new(*uri, *name).with_description(*desc).with_mime_type("application/json")
            })
            .collect();
        Ok(ListResourcesResult::with_all_items(list))
    }

    async fn list_resource_templates(
        &self,
        _: Option<PaginatedRequestParams>,
        _: RequestContext<RoleServer>,
    ) -> Result<ListResourceTemplatesResult, ErrorData> {
        let list = resources::TEMPLATES
            .iter()
            .map(|(uri, name, desc)| {
                ResourceTemplate::new(*uri, *name).with_description(*desc).with_mime_type("application/json")
            })
            .collect();
        Ok(ListResourceTemplatesResult::with_all_items(list))
    }

    async fn read_resource(
        &self,
        request: ReadResourceRequestParams,
        ctx: RequestContext<RoleServer>,
    ) -> Result<ReadResourceResponse, ErrorData> {
        let value = self.read(&Self::actor(&ctx), &request.uri).await?;
        let text = serde_json::to_string_pretty(&value).unwrap_or_default();
        let contents = ResourceContents::text(text, request.uri).with_mime_type("application/json");
        Ok(ReadResourceResult::new(vec![contents]).into())
    }

    fn accepted_subscription_filter(&self, requested: &SubscriptionFilter) -> Option<SubscriptionFilter> {
        let mut accepted = SubscriptionFilter::default();
        accepted.resource_subscriptions = requested
            .resource_subscriptions
            .as_ref()
            .map(|uris| uris.iter().filter(|u| resources::parse(u).is_some()).cloned().collect());
        Some(accepted)
    }

    async fn listen(&self, ctx: SubscriptionContext) -> Result<(), ErrorData> {
        let uris = ctx.accepted().resource_subscriptions.clone().unwrap_or_default();
        let mut events = self.shared.events();
        loop {
            tokio::select! {
                () = ctx.cancelled() => return Ok(()),
                e = events.recv() => {
                    let e = match e {
                        Ok(e) => e,
                        Err(RecvError::Lagged(_)) => continue,
                        Err(RecvError::Closed) => return Ok(()),
                    };
                    for changed in resources::changed_by(&e) {
                        for s in uris.iter().filter(|s| resources::matches(s, &changed)) {
                            if ctx.sink().notify_resource_updated(s.clone()).await.is_err() {
                                return Ok(());
                            }
                        }
                    }
                }
            }
        }
    }

    #[allow(deprecated)]
    async fn subscribe(
        &self,
        request: SubscribeRequestParams,
        ctx: RequestContext<RoleServer>,
    ) -> Result<(), ErrorData> {
        if resources::parse(&request.uri).is_none() {
            return Err(ErrorData::resource_not_found(
                format!("There is no resource {}.", request.uri),
                None,
            ));
        }
        self.legacy_subs.lock().unwrap_or_else(std::sync::PoisonError::into_inner).push(request.uri);
        self.start_legacy_updates(ctx.peer.clone());
        Ok(())
    }

    #[allow(deprecated)]
    async fn unsubscribe(
        &self,
        request: rmcp::model::UnsubscribeRequestParams,
        _: RequestContext<RoleServer>,
    ) -> Result<(), ErrorData> {
        self.legacy_subs
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .retain(|u| *u != request.uri);
        Ok(())
    }

    async fn list_prompts(
        &self,
        _: Option<PaginatedRequestParams>,
        _: RequestContext<RoleServer>,
    ) -> Result<ListPromptsResult, ErrorData> {
        let list = prompts::PROMPTS
            .iter()
            .map(|(name, desc, args)| {
                let args = args
                    .iter()
                    .map(|(a, d, req)| PromptArgument::new(*a).with_description(*d).with_required(*req))
                    .collect();
                Prompt::new(*name, Some(*desc), Some(args))
            })
            .collect();
        Ok(ListPromptsResult::with_all_items(list))
    }

    async fn get_prompt(
        &self,
        request: GetPromptRequestParams,
        _: RequestContext<RoleServer>,
    ) -> Result<GetPromptResponse, ErrorData> {
        let args = request.arguments.unwrap_or_default();
        let arg =
            |k: &str| args.get(k).and_then(Value::as_str).map(String::from).filter(|s| !s.trim().is_empty());
        let text = prompts::text(&request.name, arg).ok_or_else(|| {
            ErrorData::invalid_params(format!("There is no prompt \"{}\".", request.name), None)
        })?;
        let desc = prompts::PROMPTS.iter().find(|p| p.0 == request.name).map_or("", |p| p.1);
        Ok(GetPromptResult::new(vec![PromptMessage::new_text(Role::User, text)])
            .with_description(desc)
            .into())
    }
}
