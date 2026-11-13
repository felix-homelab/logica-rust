//! Prompts (doc 07): task recipes an agent's client can offer the person.

/// A prompt argument: name, description, whether it is required.
pub type Argument = (&'static str, &'static str, bool);

/// (name, description, arguments).
pub const PROMPTS: &[(&str, &str, &[Argument])] = &[
    (
        "diagnose-bus",
        "Find out what is on some channels, decode it, explain its problems and mark them for the person.",
        &[("channels", "The channels to look at, such as \"CH4, CH5\" or \"4,5\".", true)],
    ),
    (
        "verify-expected",
        "Check that a bus carried what the firmware should have sent, and report and mark the differences.",
        &[
            ("protocol", "The protocol, such as i2c, uart or spi.", true),
            ("expected", "The expected bytes (hex) or text.", true),
        ],
    ),
];

/// The prompt text, or `None` for an unknown prompt.
pub fn text(name: &str, arg: impl Fn(&str) -> Option<String>) -> Option<String> {
    Some(match name {
        "diagnose-bus" => {
            let channels = arg("channels").unwrap_or_else(|| "the channels in use".into());
            format!(
                "Diagnose the bus on {channels} with Logica.\n\
                 1. Call session. If no capture covers {channels}, open a file with open_file or capture with capture (configure the channels and a sample rate at least 10 times the expected bit rate).\n\
                 2. Call summarize on the capture and read the detected protocol, its settings, confidence and evidence.\n\
                 3. Add a decoder for it with decoder (action add), using the roles and settings from the detection.\n\
                 4. Call get_decoded with explain: true. If there are problems, try the setting changes the explanation suggests with decoder (action update) and compare the problem counts.\n\
                 5. Mark the first instance of each kind of problem with show_user, with a short label and the cause in the note.\n\
                 6. Report: the protocol and settings, what was sent (a few decoded packets), each problem with its time and sample, the likely cause and what to check on the hardware."
            )
        }
        "verify-expected" => {
            let protocol = arg("protocol").unwrap_or_else(|| "the protocol".into());
            let expected = arg("expected").unwrap_or_default();
            format!(
                "Check with Logica that the {protocol} bus carried this: {expected}\n\
                 1. Call session; open or capture the traffic if it is not there yet.\n\
                 2. Make sure a {protocol} decoder runs (summarize, then decoder with action add if needed).\n\
                 3. Read the decoded packets with get_decoded (use text to filter) and compare them with the expected content, byte by byte.\n\
                 4. Mark each difference with show_user: the label says what differs, the note gives expected and actual.\n\
                 5. Report whether it matches; for each difference give the time, sample, expected and actual bytes."
            )
        }
        _ => return None,
    })
}
