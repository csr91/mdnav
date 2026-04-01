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
    let docs_root = resolve_docs_root()?;
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

fn resolve_docs_root() -> Result<PathBuf> {
    let arg = env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));

    if arg.exists() {
        Ok(arg)
    } else {
        Err(anyhow::anyhow!(
            "La ruta no existe: {}. Pasa una carpeta con Markdown o ejecuta mdnav desde el directorio que quieras explorar.",
            arg.display()
        ))
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
