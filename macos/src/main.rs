mod app;
mod blocks;
mod clipboard;
mod config;
mod context;
mod dock;
mod font;
mod images;
mod input;
mod links;
mod list;
mod menu;
mod palette;
mod pane;
mod pty;
mod quake;
mod recent;
mod render;
mod restart;
mod rules;
mod search;
mod session;
mod shell;
mod suggest;
mod tap;
mod term;
mod theme;
mod welcome;

use winit::event_loop::EventLoop;

fn main() {
    env_logger::init();
    // macOS may hand an app opened from Finder its process serial number.
    let args: Vec<String> = std::env::args_os().skip(1).map(|a| a.to_string_lossy().into_owned()).filter(|a| !a.starts_with("-psn_")).collect();
    let quake = match parse(&args) {
        Ok(Cli::Window) => false,
        Ok(Cli::Quake) => true,
        Ok(Cli::Update) => std::process::exit(update()),
        Ok(Cli::Restart) => std::process::exit(restart::cli()),
        Ok(Cli::Help) => return print!("{HELP}"),
        Ok(Cli::Version) => return println!("noobsbot {}", env!("CARGO_PKG_VERSION")),
        Err(e) => {
            eprintln!("noobsbot: {e}\nRun `noobsbot --help` for the list of arguments.");
            std::process::exit(2);
        }
    };
    if quake && quake::toggle_running() {
        return; // the running one shows/hides itself
    }
    alacritty_terminal::tty::setup_env();
    shell::set_term();
    let event_loop = EventLoop::with_user_event().build().expect("create event loop");
    let mut app = app::App::new(event_loop.create_proxy(), quake);
    event_loop.run_app(&mut app).expect("run event loop");
}

const HELP: &str = concat!(
    "noobsbot ",
    env!("CARGO_PKG_VERSION"),
    "\n\nUsage: noobsbot [OPTION]

With no option, opens a new window.

Options:
  --quake          the drop-down terminal; run it again to show or hide it
  -u, --update     install the latest version
  -r, --restart    swap running noobsbot windows for fresh ones (after --update)
  -v, --version    print the version
  -h, --help       print this list

Settings:   ~/.config/noobsbot/config.toml (made on first run)
Shortcuts:  Cmd+Shift+P lists every action
More help:  https://github.com/anil0403/noobsbot-terminal
"
);

/// What the command line asks for: one option at most.
#[derive(Debug, PartialEq)]
enum Cli {
    Window,
    Quake,
    Update,
    Restart,
    Help,
    Version,
}

fn parse(args: &[String]) -> Result<Cli, String> {
    let cli = |arg: &str| match arg {
        "--quake" => Ok(Cli::Quake),
        "-u" | "--update" => Ok(Cli::Update),
        "-r" | "--restart" => Ok(Cli::Restart),
        "-h" | "--help" => Ok(Cli::Help),
        "-v" | "-V" | "--version" => Ok(Cli::Version),
        _ => Err(format!("unknown argument '{arg}'")),
    };
    match args {
        [] => Ok(Cli::Window),
        [arg] => cli(arg),
        [first, rest @ ..] => {
            cli(first)?;
            Err(match rest.iter().find(|a| cli(a).is_err()) {
                Some(a) => format!("unknown argument '{a}'"),
                None => "one option at a time".into(),
            })
        }
    }
}

/// `noobsbot --update`: rerun the installer, which fetches the latest version.
fn update() -> i32 {
    const INSTALL: &str = "curl -fsSL https://github.com/anil0403/noobsbot-terminal/raw/main/macos/install.sh | bash";
    match std::process::Command::new("bash").args(["-o", "pipefail", "-c", INSTALL]).status() {
        Ok(s) if s.success() => {
            println!("Updated. Run `noobsbot --restart` to use the new version.");
            0
        }
        Ok(s) => s.code().unwrap_or(1),
        Err(e) => {
            eprintln!("noobsbot --update needs bash and curl: {e}");
            1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arguments() {
        let parse = |args: &[&str]| parse(&args.iter().map(|a| a.to_string()).collect::<Vec<_>>());
        assert_eq!(parse(&[]), Ok(Cli::Window));
        assert_eq!(parse(&["--quake"]), Ok(Cli::Quake));
        assert_eq!((parse(&["-u"]), parse(&["--update"])), (Ok(Cli::Update), Ok(Cli::Update)));
        assert_eq!((parse(&["-r"]), parse(&["--restart"])), (Ok(Cli::Restart), Ok(Cli::Restart)));
        assert_eq!((parse(&["-h"]), parse(&["--help"])), (Ok(Cli::Help), Ok(Cli::Help)));
        for arg in ["-v", "-V", "--version"] {
            assert_eq!(parse(&[arg]), Ok(Cli::Version), "{arg}");
        }
        assert_eq!(parse(&["--gsdsdgs"]), Err("unknown argument '--gsdsdgs'".into()));
        assert_eq!(parse(&["file.txt"]), Err("unknown argument 'file.txt'".into()));
        assert_eq!(parse(&["--quake", "--nope"]), Err("unknown argument '--nope'".into()));
        assert_eq!(parse(&["--nope", "--quake"]), Err("unknown argument '--nope'".into()));
        assert_eq!(parse(&["--quake", "--update"]), Err("one option at a time".into()));
        // Every option in the help works.
        let opts = HELP.lines().filter(|l| l.starts_with("  -")).flat_map(|l| l.split_whitespace().take_while(|w| w.starts_with('-')));
        for opt in opts.map(|w| w.trim_end_matches(',')) {
            assert!(parse(&[opt]).is_ok(), "{opt}");
        }
    }
}
