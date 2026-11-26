//! `logica-vla`: run the virtual logic analyser. `logica-vla --help` explains the options.

use std::net::TcpListener;
use std::process::ExitCode;
use std::sync::Arc;

use logica_vla::Config;
use logica_vla::server::{self, Log};

const HELP: &str = "\
logica-vla: a virtual logic analyser for trying Logica's live capture without hardware.

Usage: logica-vla [options]

  --config <file>   Signals to generate, as JSON (see tools/virtual-analyser/examples).
                    Without it, one of each signal on eleven channels.
  --listen <addr>   Address to listen on (default 127.0.0.1:7171, or the file's).
  --name <text>     The analyser's name in Logica (default Bench, or the file's).
  --seed <n>        Same seed, same signals in every capture. Default: new ones each time.
  --speed <x>       1 is real time (default), 0 is as fast as Logica takes the data.
  --print-config    Print the built-in configuration as JSON and exit.
  --check           Check the configuration and exit.
  --help            This text.

In Logica the analyser appears under \"Capture live\" as \"Virtual analyser: <name>\". Logica
looks at 127.0.0.1:7171 to 7174, so up to four can run side by side with --listen; anywhere
else, start Logica with LOGICA_VLA=<host:port>[,<host:port>…].";

struct Args {
    config: Option<String>,
    listen: Option<String>,
    name: Option<String>,
    seed: Option<u64>,
    speed: Option<f64>,
    print: bool,
    check: bool,
}

fn parse(mut it: impl Iterator<Item = String>) -> Result<Option<Args>, String> {
    let mut a =
        Args { config: None, listen: None, name: None, seed: None, speed: None, print: false, check: false };
    while let Some(arg) = it.next() {
        let mut value = |name: &str| it.next().ok_or_else(|| format!("{name} needs a value."));
        match arg.as_str() {
            "--help" | "-h" => return Ok(None),
            "--config" => a.config = Some(value("--config")?),
            "--listen" => a.listen = Some(value("--listen")?),
            "--name" => a.name = Some(value("--name")?),
            "--seed" => {
                let v = value("--seed")?;
                a.seed = Some(v.parse().map_err(|_| format!("--seed takes a whole number, not \"{v}\"."))?);
            }
            "--speed" => {
                let v = value("--speed")?;
                a.speed = Some(v.parse().map_err(|_| format!("--speed takes a number, not \"{v}\"."))?);
            }
            "--print-config" => a.print = true,
            "--check" => a.check = true,
            other => return Err(format!("Unknown option \"{other}\". Try --help.")),
        }
    }
    Ok(Some(a))
}

fn load(a: &Args) -> Result<Config, String> {
    let mut c = match &a.config {
        Some(path) => {
            let text =
                std::fs::read_to_string(path).map_err(|e| format!("{path} could not be read: {e}."))?;
            serde_json::from_str::<Config>(&text)
                .map_err(|e| format!("{path} is not a valid configuration: {e}."))?
        }
        None => Config::example(),
    };
    if let Some(l) = &a.listen {
        c.listen.clone_from(l);
    }
    if let Some(n) = &a.name {
        c.name.clone_from(n);
    }
    if a.seed.is_some() {
        c.seed = a.seed;
    }
    if let Some(s) = a.speed {
        c.speed = s;
    }
    let mut problems = c.problems();
    if problems.is_empty() && c.usable_rates().is_empty() {
        problems.push(format!(
            "No rate in sampleRatesHz is fast enough: the signals need at least {} samples per second.",
            c.min_rate_hz()
        ));
    }
    if problems.is_empty() {
        Ok(c)
    } else {
        Err(format!("The configuration has problems:\n  {}", problems.join("\n  ")))
    }
}

fn main() -> ExitCode {
    let args = match parse(std::env::args().skip(1)) {
        Ok(Some(a)) => a,
        Ok(None) => {
            println!("{HELP}");
            return ExitCode::SUCCESS;
        }
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::from(2);
        }
    };
    if args.print {
        println!("{}", serde_json::to_string_pretty(&Config::example()).unwrap_or_default());
        return ExitCode::SUCCESS;
    }
    let config = match load(&args) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::from(2);
        }
    };
    let names = config.channel_names();
    let rates = config.usable_rates();
    let left_out: Vec<f64> = config.sample_rates_hz.iter().copied().filter(|r| !rates.contains(r)).collect();
    println!("Virtual analyser \"{}\"", config.name);
    for s in &config.signals {
        let chans: Vec<String> =
            s.channels().iter().map(|(c, _)| format!("CH{c} {}", names[*c as usize])).collect();
        println!("  {:<28} {}", chans.join(", "), s.describe());
    }
    println!("  rates: {}", rates.iter().map(|r| format!("{} MS/s", r / 1e6)).collect::<Vec<_>>().join(", "));
    if !left_out.is_empty() {
        println!(
            "  left out as too slow for the signals: {}",
            left_out.iter().map(|r| format!("{} MS/s", r / 1e6)).collect::<Vec<_>>().join(", ")
        );
    }
    if args.check {
        println!("The configuration is fine.");
        return ExitCode::SUCCESS;
    }
    let listener = match TcpListener::bind(&config.listen) {
        Ok(l) => l,
        Err(e) => {
            eprintln!(
                "Could not listen on {}: {e}. Is another analyser running there? Choose another with --listen.",
                config.listen
            );
            return ExitCode::FAILURE;
        }
    };
    let at = listener.local_addr().map(|a| a.to_string()).unwrap_or_else(|_| config.listen.clone());
    println!("Listening on {at}. In Logica, open the device panel; stop with Ctrl+C.");
    if !logica_drivers::vla::DEFAULT_ADDRESSES.contains(&at.as_str()) {
        println!("Start Logica with LOGICA_VLA={at} so it looks here.");
    }
    let log: Log = Arc::new(|m: &str| println!("{m}"));
    match server::serve(listener, config, log) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("The analyser stopped: {e}.");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Result<Option<Args>, String> {
        parse(list.iter().map(|s| s.to_string()))
    }

    #[test]
    fn options_are_read_and_mistakes_explained() {
        let a = args(&["--seed", "4", "--speed", "0", "--listen", "127.0.0.1:0"]).unwrap().unwrap();
        assert_eq!((a.seed, a.speed, a.listen.as_deref()), (Some(4), Some(0.0), Some("127.0.0.1:0")));
        let c = load(&a).unwrap();
        assert_eq!((c.seed, c.speed, c.listen.as_str()), (Some(4), 0.0, "127.0.0.1:0"));
        assert!(args(&["--help"]).unwrap().is_none());
        assert_eq!(args(&["--seed"]).err().unwrap(), "--seed needs a value.");
        assert!(args(&["--seed", "x"]).err().unwrap().contains("whole number"));
        assert!(args(&["--fast"]).err().unwrap().contains("Unknown option \"--fast\""));
        let bad = args(&["--speed", "-2"]).unwrap().unwrap();
        assert!(load(&bad).unwrap_err().contains("speed must be 0"));
        let named = args(&["--name", "Second bench"]).unwrap().unwrap();
        assert_eq!(load(&named).unwrap().name, "Second bench");
        let blank = args(&["--name", " "]).unwrap().unwrap();
        assert!(load(&blank).unwrap_err().contains("Give the analyser a name."));
        assert_eq!(args(&["--name"]).err().unwrap(), "--name needs a value.");
        let missing = args(&["--config", "/nonexistent/vla.json"]).unwrap().unwrap();
        assert!(load(&missing).unwrap_err().contains("could not be read"));
    }

    #[test]
    fn the_example_files_are_valid() {
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/examples");
        let mut n = 0;
        for f in std::fs::read_dir(dir).unwrap() {
            let path = f.unwrap().path();
            if path.extension().is_some_and(|e| e == "json") {
                let a = Args {
                    config: Some(path.display().to_string()),
                    name: None,
                    listen: None,
                    seed: None,
                    speed: None,
                    print: false,
                    check: true,
                };
                load(&a).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
                n += 1;
            }
        }
        assert!(n >= 2);
    }
}
