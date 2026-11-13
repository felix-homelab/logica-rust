//! The `logica` command (doc 03).

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    // not locked: `logica mcp` writes to stdout from the MCP transport's own thread
    let code = logica_cli::run(&args, &mut std::io::stdout(), &mut std::io::stderr());
    std::process::exit(code);
}
