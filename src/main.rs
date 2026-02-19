use anyhow::Result;
use crossterm::{
    ExecutableCommand,
    event::{self, Event, KeyCode, KeyEvent, KeyEventKind},
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{
    Frame, Terminal,
    backend::{Backend, CrosstermBackend},
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Tabs, Wrap},
};
use std::io::{Stdout, stdout};

mod app;
mod contexts;
mod palette;
mod systemd;
mod widgets;

use app::App;
use contexts::Context;

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize tracing
    tracing_subscriber::fmt::init();

    // Setup terminal
    let mut terminal = setup_terminal()?;

    // Create app (async - connects to systemd)
    let mut app = match App::new().await {
        Ok(app) => app,
        Err(e) => {
            eprintln!("Failed to initialize: {}", e);
            return Err(e);
        }
    };

    // Run app
    let result = run_app(&mut terminal, &mut app).await;

    // Restore terminal
    restore_terminal(terminal)?;

    result
}

fn setup_terminal() -> Result<Terminal<CrosstermBackend<Stdout>>> {
    let mut stdout = stdout();
    enable_raw_mode()?;
    stdout.execute(EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let terminal = Terminal::new(backend)?;
    Ok(terminal)
}

fn restore_terminal(mut terminal: Terminal<CrosstermBackend<Stdout>>) -> Result<()> {
    disable_raw_mode()?;
    terminal.backend_mut().execute(LeaveAlternateScreen)?;
    Ok(())
}

async fn run_app<B: Backend>(terminal: &mut Terminal<B>, app: &mut App) -> Result<()> {
    let mut last_tick = std::time::Instant::now();
    let tick_rate = std::time::Duration::from_millis(250);
    let refresh_interval = std::time::Duration::from_secs(2);
    let mut last_refresh = std::time::Instant::now();

    loop {
        terminal.draw(|f| draw(f, app))?;

        let timeout = tick_rate
            .checked_sub(last_tick.elapsed())
            .unwrap_or_else(|| std::time::Duration::from_secs(0));

        if crossterm::event::poll(timeout)? {
            if let Event::Key(key) = event::read()? {
                if key.kind == KeyEventKind::Press {
                    match handle_key(key, app) {
                        Action::Continue => {}
                        Action::Quit => break,
                    }
                }
            }
        }

        if last_tick.elapsed() >= tick_rate {
            app.tick().await;
            last_tick = std::time::Instant::now();
        }

        // Periodic refresh every 2 seconds
        if last_refresh.elapsed() >= refresh_interval {
            last_refresh = std::time::Instant::now();
        }
    }

    Ok(())
}

enum Action {
    Continue,
    Quit,
}

fn handle_key(key: KeyEvent, app: &mut App) -> Action {
    match key.code {
        KeyCode::Char('q') | KeyCode::Char('Q') => return Action::Quit,
        KeyCode::Char('?') => app.toggle_help(),
        KeyCode::Tab => app.next_context(),
        KeyCode::BackTab => app.prev_context(),
        KeyCode::Char('1') => app.set_context(0),
        KeyCode::Char('2') => app.set_context(1),
        KeyCode::Char('3') => app.set_context(2),
        KeyCode::Char('4') => app.set_context(3),
        KeyCode::Char('5') => app.set_context(4),
        KeyCode::Char('6') => app.set_context(5),
        _ => app.handle_key(key),
    }
    Action::Continue
}

fn draw(f: &mut Frame, app: &App) {
    // Main layout
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .margin(0)
        .constraints([
            Constraint::Length(3), // Header with tabs
            Constraint::Min(10),   // Main content
            Constraint::Length(1), // Status line
        ])
        .split(f.area());

    // Header with tabs
    draw_header(f, app, chunks[0]);

    // Main content area - delegate to current context
    draw_content(f, app, chunks[1]);

    // Status line
    draw_status(f, app, chunks[2]);

    // Help overlay if active
    if app.show_help() {
        draw_help(f, app);
    }
}

fn draw_header(f: &mut Frame, app: &App, area: Rect) {
    let header_layout = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(12), Constraint::Min(20)])
        .split(area);

    // Title block with user mode indicator
    let mode_indicator = if app.systemd().is_user_mode() {
        "[user]"
    } else {
        "[system]"
    };
    let title_text = format!("🐾 rootwork\n{}", mode_indicator);
    let title = Paragraph::new(title_text)
        .style(
            Style::default()
                .fg(crate::palette::cyan())
                .add_modifier(Modifier::BOLD),
        )
        .block(Block::default().borders(Borders::ALL));
    f.render_widget(title, header_layout[0]);

    // Tabs
    let titles = vec![
        "[1] Units",
        "[2] Network",
        "[3] DNS",
        "[4] Host",
        "[5] Boot",
        "[6] Logs",
    ];
    let tabs = Tabs::new(titles)
        .select(app.current_context())
        .style(Style::default().fg(crate::palette::white()))
        .highlight_style(
            Style::default()
                .fg(crate::palette::green())
                .add_modifier(Modifier::BOLD),
        )
        .divider(" | ")
        .block(Block::default().borders(Borders::ALL));
    f.render_widget(tabs, header_layout[1]);
}

fn draw_content(f: &mut Frame, app: &App, area: Rect) {
    match app.current_context() {
        0 => app.units().draw(f, area),
        1 => app.network().draw(f, area),
        2 => app.dns().draw(f, area),
        3 => app.host().draw(f, area),
        4 => app.boot().draw(f, area),
        5 => app.logs().draw(f, area),
        _ => {
            let block = Block::default()
                .borders(Borders::ALL)
                .title(" Unknown Context ");
            let content = Paragraph::new("Unknown context").block(block);
            f.render_widget(content, area);
        }
    }
}

fn draw_status(f: &mut Frame, app: &App, area: Rect) {
    let mode_str = if app.systemd().is_user_mode() {
        "[user]"
    } else {
        "[system]"
    };

    let status = Line::from(vec![
        Span::raw(format!("{} ", mode_str)),
        Span::raw("j:down k:up sp:pg t:view s:sort e:xpnd c:clps /:fltr r:ref ?:help "),
        Span::styled(
            "q:quit",
            Style::default()
                .fg(crate::palette::red())
                .add_modifier(Modifier::BOLD),
        ),
    ]);
    let status_bar = Paragraph::new(status);
    f.render_widget(status_bar, area);
}

fn draw_help(f: &mut Frame, app: &App) {
    let help_text = match app.current_context() {
        0 => {
            r#"Units View (Tree mode default):
    j, ↓          Down        k, ↑          Up
    g             Top         G             Bottom
    Space, PgDn   Page down   b, PgUp       Page up
    /             Filter      Esc           Clear filter
    Enter         Toggle group expand/collapse
    e             Expand all  c             Collapse all
    t             Toggle tree/list view
    s             Toggle sort (name/state)
    S             Toggle sort direction"#
        }

        1 => {
            r#"Network View:
    j, ↓          Down        k, ↑          Up
    r             Refresh"#
        }

        2 => {
            r#"DNS View:
    j, ↓          Down        k, ↑          Up
    r             Refresh"#
        }

        3 => {
            r#"Host View:
    r             Refresh host information"#
        }

        4 => {
            r#"Boot View:
    j, ↓          Down        k, ↑          Up
    r             Refresh"#
        }

        5 => {
            r#"Logs View:
    j, ↓          Down        k, ↑          Up
    g             Top         G             Bottom (follow)
    Space, PgDn   Page down   b, PgUp       Page up
    p             Pause/unpause streaming
    f             Toggle follow mode
    c             Clear logs
    r             Refresh/reload"#
        }

        _ => "Unknown context",
    };

    let global_help = r#"

Global:
    q, Q          Quit
    ?             Toggle this help
    Tab           Next context
    Shift+Tab     Previous context
    1-6           Jump to context

Press any key to close this help"#;

    let full_help = format!("{}{}", help_text, global_help);

    let block = Block::default()
        .title(format!(" Help - {} ", app.context_name()))
        .borders(Borders::ALL)
        .style(Style::default().bg(crate::palette::black()));

    let help = Paragraph::new(full_help)
        .block(block)
        .wrap(Wrap { trim: true });

    // Center the help popup
    let area = centered_rect(70, 80, f.area());
    f.render_widget(help, area);
}

fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(r);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1]
}
