// The Deck app starts the plugin in the background; on Windows a console
// program would open a stray terminal window, so build a GUI-subsystem exe.
#![cfg_attr(windows, windows_subsystem = "windows")]

use std::process::ExitCode;

fn main() -> ExitCode {
    let args = match ts3deck::deck::parse_args(std::env::args().skip(1)) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::from(2);
        }
    };
    match ts3deck::deck::run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("ts3deck: {e}");
            ExitCode::FAILURE
        }
    }
}
