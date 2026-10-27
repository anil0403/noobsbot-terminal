mod app;
mod blocks;
mod clipboard;
mod config;
mod context;
mod font;
mod images;
mod input;
mod kitty;
mod links;
mod list;
mod palette;
mod pane;
mod pty;
mod quake;
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
    let (quake, exec) = match parse(&args) {
        Ok(Cli::Window) => (false, None),
        Ok(Cli::Quake) => (true, None),
        Ok(Cli::Exec(cmd)) => (false, Some(cmd)),
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
    let mut app = app::App::new(event_loop.create_proxy(), quake, exec);
    event_loop.run_app(&mut app).expect("run event loop");
}

const HELP: &str = concat!(
    "noobsbot ",
    env!("CARGO_PKG_VERSION"),
    "\n\nUsage: noobsbot [OPTION]
       noobsbot -e PROGRAM [ARGS...]

With no option, opens a new window. With -e (or --), the window runs PROGRAM
instead of the shell; everything after it is PROGRAM's arguments.

Options:
  --quake          the drop-down terminal; run it again (or press F12) to show or hide it
  -u, --update     install the latest version
  -r, --restart    swap running noobsbot windows for fresh ones (after --update)
  -v, --version    print the version
  -h, --help       print this list

Settings:   ~/.config/noobsbot/config.toml (made on first run)
Shortcuts:  Ctrl+Shift+P lists every action
More help:  https://github.com/anil0403/noobsbot-terminal
"
);

/// What the command line asks for: one option at most.
#[derive(Debug, PartialEq)]
enum Cli {
    Window,
    Quake,
    /// `-e program args…`: a window running that instead of the shell.
    Exec(Vec<String>),
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
    // Everything after `-e` (or `--`) is the command, flags and all.
    if let Some(i) = args.iter().position(|a| a == "-e" || a == "--") {
        if i + 1 == args.len() {
            return Err(format!("'{}' needs a program to run", args[i]));
        }
        return match parse(&args[..i])? {
            Cli::Window => Ok(Cli::Exec(args[i + 1..].to_vec())),
            Cli::Quake => Err("--quake can't run a program".into()),
            _ => Err("one option at a time".into()),
        };
    }
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

/// `noobsbot --update`: rerun the installer, which installs the latest version.
fn update() -> i32 {
    const INSTALL: &str = "curl -fsSL https://github.com/anil0403/noobsbot-terminal/raw/main/install.sh | bash";
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
        let exec = |cmd: &[&str]| Ok(Cli::Exec(cmd.iter().map(|a| a.to_string()).collect()));
        assert_eq!(parse(&["-e", "htop"]), exec(&["htop"]));
        assert_eq!(parse(&["--", "htop"]), exec(&["htop"]));
        assert_eq!(parse(&["-e", "vim", "-e", "--help", "--", "a b"]), exec(&["vim", "-e", "--help", "--", "a b"]));
        assert_eq!(parse(&["-e"]), Err("'-e' needs a program to run".into()));
        assert_eq!(parse(&["--"]), Err("'--' needs a program to run".into()));
        assert_eq!(parse(&["--quake", "-e", "htop"]), Err("--quake can't run a program".into()));
        assert_eq!(parse(&["--update", "-e", "htop"]), Err("one option at a time".into()));
        assert_eq!(parse(&["--nope", "-e", "htop"]), Err("unknown argument '--nope'".into()));
        // Every option in the help works.
        let opts = HELP.lines().filter(|l| l.starts_with("  -")).flat_map(|l| l.split_whitespace().take_while(|w| w.starts_with('-')));
        for opt in opts.map(|w| w.trim_end_matches(',')) {
            assert!(parse(&[opt]).is_ok(), "{opt}");
        }
    }
}
