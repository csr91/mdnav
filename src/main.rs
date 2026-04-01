mod app;
mod config;
mod docs;
mod markdown;
mod ui;

use std::{env, io, path::PathBuf, process::Command};

use anyhow::{Context, Result};
use crossterm::{
    event::{self, Event},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{backend::CrosstermBackend, Terminal};

use crate::{app::App, config::AppConfig};

fn main() -> Result<()> {
    match resolve_command()? {
        CliCommand::Run { docs_root } => run_cli(docs_root),
        CliCommand::ShellHook { shell } => {
            print!("{}", shell_hook_script(&shell)?);
            Ok(())
        }
    }
}

fn run_cli(docs_root: PathBuf) -> Result<()> {
    let config = AppConfig::load()?;
    let mut resume_path: Option<PathBuf> = None;

    loop {
        let mut terminal = setup_terminal()?;
        let result = run(&mut terminal, docs_root.clone(), config.clone(), resume_path.take());
        restore_terminal(&mut terminal)?;
        let app = result?;

        if let Some(path) = app.pending_external_edit.clone() {
            open_in_nano(&path)?;
            resume_path = Some(path);
            continue;
        }

        emit_pending_cd(&app);
        return Ok(());
    }
}

enum CliCommand {
    Run { docs_root: PathBuf },
    ShellHook { shell: String },
}

fn resolve_command() -> Result<CliCommand> {
    let args = env::args().skip(1).collect::<Vec<_>>();

    match args.as_slice() {
        [] => Ok(CliCommand::Run {
            docs_root: PathBuf::from("."),
        }),
        [flag, shell] if flag == "--shell-hook" => Ok(CliCommand::ShellHook {
            shell: shell.to_string(),
        }),
        [path] => {
            let docs_root = PathBuf::from(path);
            if docs_root.exists() {
                Ok(CliCommand::Run { docs_root })
            } else {
                Err(anyhow::anyhow!(
                    "La ruta no existe: {}. Pasa una carpeta con Markdown o ejecuta mdnav desde el directorio que quieras explorar.",
                    docs_root.display()
                ))
            }
        }
        _ => Err(anyhow::anyhow!(
            "Uso: mdnav [ruta] | mdnav --shell-hook <bash|zsh>"
        )),
    }
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
    terminal.show_cursor().context("No se pudo restaurar el cursor")
}

fn run(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    docs_root: PathBuf,
    config: AppConfig,
    resume_path: Option<PathBuf>,
) -> Result<App> {
    let mut app = App::new(docs_root, config)?;

    if let Some(path) = resume_path {
        app.restore_path_focus(&path)?;
    }

    while app.running {
        terminal.draw(|frame| ui::render(frame, &app))?;

        if let Event::Key(key_event) = event::read()? {
            app.handle_key(key_event)?;
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

fn open_in_nano(path: &PathBuf) -> Result<()> {
    let status = Command::new("nano").arg(path).status()?;
    if status.success() {
        Ok(())
    } else {
        Err(anyhow::anyhow!("nano termino con estado {status}"))
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
        _ => Err(anyhow::anyhow!(
            "Shell no soportada: {shell}. Usa bash o zsh."
        )),
    }
}
