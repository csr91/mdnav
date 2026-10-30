mod app;
mod config;
mod docs;
mod http;
mod markdown;
mod postman;
mod share;
mod strings;
mod studio;
mod ui;

use std::{
    env,
    io::{self, Read as _},
    path::{Path, PathBuf},
    process::Command,
    time::Duration,
};

use anyhow::{Context, Result};
use crossterm::{
    event::{self, poll, Event},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{backend::CrosstermBackend, Terminal};

use crate::{
    app::{App, HttpSession},
    config::AppConfig,
};

fn main() -> Result<()> {
    install_panic_hook();
    match resolve_command()? {
        CliCommand::Run { docs_root } => run_cli(docs_root),
        CliCommand::ShellHook { shell } => {
            print!("{}", shell_hook_script(&shell)?);
            Ok(())
        }
        CliCommand::Share {
            source,
            title,
            ttl_seconds,
            open,
        } => run_share(source, title, ttl_seconds, open),
    }
}

fn run_cli(docs_root: PathBuf) -> Result<()> {
    let mut config = AppConfig::load()?;
    let mut resume_path: Option<PathBuf> = None;
    let mut current_root = docs_root;
    let mut http_session = HttpSession::default();

    loop {
        let mut terminal = setup_terminal()?;
        let result = run(
            &mut terminal,
            current_root.clone(),
            config.clone(),
            resume_path.take(),
            std::mem::take(&mut http_session),
        );
        restore_terminal(&mut terminal)?;
        let mut app = result?;

        if let Some(path) = app.pending_external_edit.clone() {
            current_root = app.root.clone();
            // The editor round-trip relaunches the app: carry over what changed
            // in this session (settings, recents, active environments) and the
            // HTTP responses, including a request still on the wire.
            config = app.config.clone();
            http_session = std::mem::take(&mut app.http.session);
            open_in_editor(&path, &app.config.editor)?;
            resume_path = Some(path);
            continue;
        }

        emit_pending_cd(&app);
        return Ok(());
    }
}

enum CliCommand {
    Run {
        docs_root: PathBuf,
    },
    ShellHook {
        shell: String,
    },
    Share {
        source: ShareSource,
        title: Option<String>,
        ttl_seconds: Option<u64>,
        open: bool,
    },
}

enum ShareSource {
    File(PathBuf),
    Stdin,
    Clipboard,
}

const SHARE_USAGE: &str =
    "Uso: mdnav share --file <ruta> | --stdin | --clipboard [--title <texto>] [--ttl <segundos>] [--open]";

fn resolve_command() -> Result<CliCommand> {
    let args = env::args().skip(1).collect::<Vec<_>>();

    match args.as_slice() {
        [] => Ok(CliCommand::Run {
            docs_root: env::current_dir().context("No se pudo resolver el directorio actual")?,
        }),
        [flag] if flag == "--version" || flag == "-V" => {
            println!("mdnav {}", env!("CARGO_PKG_VERSION"));
            std::process::exit(0);
        }
        [flag, shell] if flag == "--shell-hook" => Ok(CliCommand::ShellHook {
            shell: shell.to_string(),
        }),
        [flag, rest @ ..] if flag == "share" => parse_share_args(rest),
        [path] => {
            let docs_root = PathBuf::from(path);
            if docs_root.exists() {
                let docs_root = if docs_root.is_absolute() {
                    docs_root
                } else {
                    env::current_dir()?.join(&docs_root)
                };
                Ok(CliCommand::Run { docs_root })
            } else {
                Err(anyhow::anyhow!(
                    "La ruta no existe: {}. Pasa una carpeta con Markdown o ejecuta mdnav desde el directorio que quieras explorar.",
                    docs_root.display()
                ))
            }
        }
        _ => Err(anyhow::anyhow!(
            "Uso: mdnav [ruta] | mdnav --shell-hook <bash|zsh> | {SHARE_USAGE}"
        )),
    }
}

fn parse_share_args(args: &[String]) -> Result<CliCommand> {
    let mut source: Option<ShareSource> = None;
    let mut title: Option<String> = None;
    let mut ttl_seconds: Option<u64> = None;
    let mut open = false;

    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--file" => {
                let path = iter.next().context(SHARE_USAGE)?;
                source = Some(ShareSource::File(PathBuf::from(path)));
            }
            "--stdin" => source = Some(ShareSource::Stdin),
            "--clipboard" => source = Some(ShareSource::Clipboard),
            "--title" => {
                title = Some(iter.next().context(SHARE_USAGE)?.to_string());
            }
            "--ttl" => {
                let raw = iter.next().context(SHARE_USAGE)?;
                ttl_seconds = Some(
                    raw.parse::<u64>()
                        .with_context(|| format!("--ttl invalido: {raw}"))?,
                );
            }
            "--open" => open = true,
            other => {
                return Err(anyhow::anyhow!(
                    "Argumento desconocido: {other}\n{SHARE_USAGE}"
                ))
            }
        }
    }

    let source = source.context(SHARE_USAGE)?;
    Ok(CliCommand::Share {
        source,
        title,
        ttl_seconds,
        open,
    })
}

fn run_share(
    source: ShareSource,
    title: Option<String>,
    ttl_seconds: Option<u64>,
    open: bool,
) -> Result<()> {
    let (content, kind, filename) = match source {
        ShareSource::File(path) => {
            let content = std::fs::read_to_string(&path)
                .with_context(|| format!("No se pudo leer {}", path.display()))?;
            let kind = share::infer_kind_from_extension(&path);
            let filename = path
                .file_name()
                .map(|name| name.to_string_lossy().to_string());
            (content, kind, filename)
        }
        ShareSource::Stdin => {
            let mut content = String::new();
            io::stdin()
                .read_to_string(&mut content)
                .context("No se pudo leer stdin")?;
            (content, "text", None)
        }
        ShareSource::Clipboard => {
            let mut clipboard =
                arboard::Clipboard::new().context("No se pudo acceder al portapapeles")?;
            let content = clipboard
                .get_text()
                .context("El portapapeles no tiene texto")?;
            (content, "text", None)
        }
    };

    let url = share::publish_share(
        kind,
        &content,
        title.as_deref(),
        filename.as_deref(),
        ttl_seconds,
    )?;

    let copied = app::copy_to_clipboard(&url).unwrap_or(false);
    if open {
        app::open_url_in_browser(&url)?;
    }

    println!("{url}");
    if copied {
        eprintln!("Link copiado al portapapeles");
    }

    Ok(())
}

/// Restores the terminal before a panic prints, so an unexpected crash never
/// leaves the user's shell in raw mode / alternate screen. Best-effort: errors
/// during restore are ignored because we are already unwinding.
fn install_panic_hook() {
    let original_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |panic_info| {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen);
        original_hook(panic_info);
    }));
}

fn setup_terminal() -> Result<Terminal<CrosstermBackend<io::Stdout>>> {
    enable_raw_mode().context("No se pudo activar raw mode")?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen).context("No se pudo entrar en alternate screen")?;
    let backend = CrosstermBackend::new(stdout);
    Terminal::new(backend).context("No se pudo inicializar la terminal")
}

fn restore_terminal(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>) -> Result<()> {
    disable_raw_mode().context("No se pudo desactivar raw mode")?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)
        .context("No se pudo salir de alternate screen")?;
    terminal
        .show_cursor()
        .context("No se pudo restaurar el cursor")
}

fn run(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    docs_root: PathBuf,
    config: AppConfig,
    resume_path: Option<PathBuf>,
    http_session: HttpSession,
) -> Result<App> {
    let mut app = App::new(docs_root, config)?;
    app.http.session = http_session;

    if let Some(path) = resume_path {
        app.restore_path_focus(&path)?;
    }
    if let Some(path) = app.http.session.reopen_studio.take() {
        app.reopen_http_studio(&path)?;
    }

    while app.running {
        app.poll_http();
        terminal.draw(|frame| ui::render(frame, &app))?;

        // Tick faster while a request is on the wire so the spinner moves and
        // the response shows up as soon as it lands.
        let tick = if app.http_busy() {
            Duration::from_millis(100)
        } else {
            Duration::from_millis(500)
        };

        // A failing action (permission denied, file vanished, ...) must never
        // tear down the TUI: surface it as an on-screen alert and keep going.
        if poll(tick)? {
            if let Event::Key(key_event) = event::read()? {
                if let Err(err) = app.handle_key(key_event) {
                    app.show_alert(format!("{err:#}"));
                }
            }
        } else if let Err(err) = app.check_external_changes() {
            app.show_alert(format!("{err:#}"));
        }
    }

    Ok(app)
}

fn emit_pending_cd(app: &App) {
    let Some(target) = app.pending_cd.as_ref() else {
        return;
    };

    if let Ok(file_path) = env::var("MDNAV_CD_FILE") {
        let _ = std::fs::write(file_path, target.display().to_string());
        return;
    }

    println!("mdnav pending cd: {}", target.display());
    println!("Run this in your shell: cd \"{}\"", target.display());
}

fn open_in_editor(path: &Path, editor: &str) -> Result<()> {
    let path_str = path.to_string_lossy();
    let path_str = path_str.trim_start_matches(r"\\?\");
    let status = Command::new(editor).arg(path_str).status()?;
    if status.success() {
        Ok(())
    } else {
        Err(anyhow::anyhow!("{editor} termino con estado {status}"))
    }
}

fn shell_hook_script(shell: &str) -> Result<String> {
    match shell {
        "bash" | "zsh" => Ok(String::from(
            r#"mdnav() {
  local tmp exit_code target
  tmp="$(mktemp "${TMPDIR:-/tmp}/mdnav-cd.XXXXXX")" || return 1
  MDNAV_CD_FILE="$tmp" command mdnav "$@"
  exit_code=$?
  if [ -s "$tmp" ]; then
    target="$(cat "$tmp")"
    rm -f "$tmp"
    if [ -n "$target" ]; then
      cd "$target" || return $exit_code
    fi
  else
    rm -f "$tmp"
  fi
  return $exit_code
}
"#,
        )),
        "powershell" => Ok(String::from(
            r#"function mdnav {
  $tmp = [System.IO.Path]::GetTempFileName()
  $env:MDNAV_CD_FILE = $tmp
  & "$env:LOCALAPPDATA\mdnav\bin\mdnav.exe" @args
  Remove-Item Env:\MDNAV_CD_FILE -ErrorAction SilentlyContinue
  if ((Test-Path $tmp) -and (Get-Item $tmp).Length -gt 0) {
    $target = (Get-Content $tmp -Raw).Trim()
    Remove-Item $tmp -Force
    if ($target) { Set-Location $target }
  } else {
    Remove-Item $tmp -ErrorAction SilentlyContinue
  }
}
"#,
        )),
        _ => Err(anyhow::anyhow!(
            "Shell no soportada: {shell}. Usa bash, zsh o powershell."
        )),
    }
}
