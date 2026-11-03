#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

//! `recalc`: with a subcommand, a CLI over the board in the current directory;
//! with none, the desktop app on that board. One binary, so the MCP config an
//! agent cell writes (`current_exe() mcp`) points back at it.

mod board;
#[cfg(debug_assertions)]
mod bridge;
mod commands;
mod export;
mod library;
mod menu;
mod native;
mod pty;
mod update;

fn main() {
    recalc::crash::install();
    let args = recalc::cli::parse();
    let Some(cmd) = args.cmd else {
        board::run(board::launch_dir());
        return;
    };
    // Like any command-line tool, end quietly when the reader goes
    // (`recalc get … | head`), rather than panicking on the closed pipe.
    #[cfg(unix)]
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_DFL);
    }
    // The commands and agents a subcommand starts reach this same build as `recalc`.
    recalc::shell_env::put_own_dir_on_path();
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    let open = match rt.block_on(recalc::cli::run(cmd)) {
        Ok(open) => open,
        Err(e) => {
            eprintln!("recalc: {e:#}");
            std::process::exit(1);
        }
    };
    drop(rt);
    // `recalc demo` ends by opening a window.
    if let Some(dir) = open {
        board::run(dir);
    }
}
