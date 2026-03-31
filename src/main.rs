mod app;
mod config;
mod docs;
mod markdown;
mod ui;

use std::{env, io, path::PathBuf};

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
    let mut terminal = setup_terminal()?;
    let result = run(&mut terminal, docs_root);
    restore_terminal(&mut terminal)?;
    result
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

fn run(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>, docs_root: PathBuf) -> Result<()> {
    let config = AppConfig::load()?;
    let mut app = App::new(docs_root, config)?;

    while app.running {
        terminal.draw(|frame| ui::render(frame, &app))?;

        if let Event::Key(key_event) = event::read()? {
            app.handle_key(key_event)?;
        }
    }

    Ok(())
}
