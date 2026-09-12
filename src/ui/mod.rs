// ── UI (presentación) ────────────────────────────────────────────────
// F2: dos vistas — resumen (F0/F1) y churn con barras de bloques.
// El consumo del engine es síncrono en `r` todavía; el worker + caché
// llega en F3. Nada de operaciones: corte de cordón (DECISIONS §2A).

use std::io::{self, Stdout};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind, KeyModifiers},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{
    Frame, Terminal,
    backend::CrosstermBackend,
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
};

use gadv::config::{self, Config};
use gadv::engine::{self, History, Window, churn};
use gadv::log;
use gadv::theme::{self, Theme};
use gadv::version;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum View {
    Summary,
    Churn,
}

pub struct App {
    pub repo_path: PathBuf,
    pub theme: Theme,
    pub no_cache: bool,
    pub max_commits: u64,
    pub repo_name: String,
    pub head: String,
    pub commits: String,
    pub status: String,
    pub scans: u32,
    pub view: View,
    /// (path, churn, touches) top-20 ya ordenado, listo para pintar.
    pub churn_rows: Vec<(String, u32, u32)>,
    history: Option<History>,
}

impl App {
    fn new(repo_path: PathBuf, cfg: &Config, no_cache: bool) -> Self {
        Self {
            repo_path,
            theme: theme::get_theme_by_name(&cfg.theme),
            no_cache,
            max_commits: cfg.max_commits,
            repo_name: String::new(),
            head: String::new(),
            commits: String::new(),
            status: String::from("listo"),
            scans: 0,
            view: View::Summary,
            churn_rows: Vec::new(),
            history: None,
        }
    }

    fn refresh(&mut self) {
        self.repo_name = engine::repo_name(&self.repo_path);
        self.head = engine::head_raw(&self.repo_path);
        let t0 = Instant::now();
        match engine::scan_history(&self.repo_path, self.max_commits) {
            Ok(h) => {
                let rows = churn(&h, Window::ALL);
                self.churn_rows = rows
                    .into_iter()
                    .take(20)
                    .map(|r| {
                        let path = h
                            .paths
                            .get(r.file.0 as usize)
                            .cloned()
                            .unwrap_or_else(|| format!("#{}", r.file.0));
                        (path, r.churn(), r.touches)
                    })
                    .collect();
                self.commits = format!("{} commits en {} ms", h.commits.len(), t0.elapsed().as_millis());
                self.history = Some(h);
            }
            Err(err) => {
                self.commits = err.to_string();
                self.churn_rows.clear();
                self.history = None;
            }
        }
        self.scans += 1;
        self.status = format!("refrescado ×{}", self.scans);
    }
}

/// Launch the TUI. Restaura el terminal incluso si el loop devuelve Err.
pub fn run(repo_path: &Path, debug: bool, no_cache: bool) -> Result<(), Box<dyn std::error::Error>> {
    let cfg = config::load_config();
    if debug {
        log::log_debug(&format!("config: theme={}", cfg.theme));
    }

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    install_panic_hook();

    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut app = App::new(repo_path.to_path_buf(), &cfg, no_cache);
    app.refresh();

    let result = event_loop(&mut terminal, &mut app, debug);

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    result
}

/// Si hay panic dentro del loop, el terminal no queda crudo: se restaura
/// antes de imprimir el backtrace.
fn install_panic_hook() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let default = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            let _ = disable_raw_mode();
            let _ = execute!(io::stdout(), LeaveAlternateScreen);
            default(info);
        }));
    });
}

fn event_loop(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    app: &mut App,
    debug: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut first = true;
    loop {
        terminal.draw(|f| draw(f, app))?;
        if debug && first {
            log::log_debug("primer frame dibujado");
            first = false;
        }
        if event::poll(Duration::from_millis(100))?
            && let Event::Key(key) = event::read()?
        {
            if key.kind != KeyEventKind::Press {
                continue;
            }
            match key.code {
                KeyCode::Char('q') => return Ok(()),
                KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    return Ok(());
                }
                KeyCode::Esc => match app.view {
                    View::Churn => app.view = View::Summary,
                    View::Summary => return Ok(()),
                },
                KeyCode::Char('1') => app.view = View::Summary,
                KeyCode::Char('2') => {
                    if app.history.is_some() {
                        app.view = View::Churn;
                    }
                }
                KeyCode::Char('r') => {
                    let t0 = Instant::now();
                    app.refresh();
                    if debug {
                        log::log_debug(&format!("refresh en {:?}", t0.elapsed()));
                    }
                }
                _ => {}
            }
        }
    }
}

fn draw(f: &mut Frame, app: &App) {
    let area = f.area();
    let t = &app.theme;
    f.render_widget(Paragraph::new("").style(Style::default().bg(t.background)), area);
    match app.view {
        View::Summary => draw_summary(f, app, area),
        View::Churn => draw_churn(f, app, area),
    }
}

fn centered(area: Rect, min_w: u16, max_w: u16, min_h: u16, max_h: u16) -> Rect {
    let width = area.width.saturating_sub(2).clamp(min_w, max_w).min(area.width);
    let height = area.height.saturating_sub(2).clamp(min_h, max_h).min(area.height);
    Rect {
        x: area.x + (area.width - width) / 2,
        y: area.y + (area.height - height) / 2,
        width,
        height,
    }
}

fn draw_summary(f: &mut Frame, app: &App, area: Rect) {
    let t = &app.theme;
    let outer = centered(area, 36, 72, 10, 15);
    draw_solid_border(f, outer, t);

    let label = |s: &'static str| Span::styled(s, Style::default().fg(t.dimmed));
    let lines = vec![
        Line::from(vec![
            Span::styled(
                " git-advance ",
                Style::default()
                    .fg(t.background)
                    .bg(t.primary)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(format!("  {}", version::full()), Style::default().fg(t.dimmed)),
        ]),
        Line::from(""),
        Line::from(vec![
            label("repo   "),
            Span::styled(app.repo_name.clone(), Style::default().fg(t.foreground)),
        ]),
        Line::from(vec![
            label("       "),
            Span::styled(
                app.repo_path.display().to_string(),
                Style::default().fg(t.dimmed),
            ),
        ]),
        Line::from(vec![
            label("head   "),
            Span::styled(app.head.clone(), Style::default().fg(t.success)),
        ]),
        Line::from(vec![
            label("commits"),
            Span::styled(" ".to_string() + &app.commits, Style::default().fg(t.primary)),
        ]),
        Line::from(vec![
            label("cache  "),
            Span::styled(
                if app.no_cache { "off (--no-cache)" } else { "on" },
                Style::default().fg(t.accent),
            ),
        ]),
        Line::from(""),
        Line::from(Span::styled(
            "1 resumen · 2 churn · r refrescar · q/Esc salir",
            Style::default().fg(t.dimmed),
        )),
        Line::from(Span::styled(app.status.clone(), Style::default().fg(t.warning))),
    ];

    let inner = Rect {
        x: outer.x + 2,
        y: outer.y + 1,
        width: outer.width.saturating_sub(4),
        height: outer.height.saturating_sub(2),
    };
    f.render_widget(
        Paragraph::new(lines).style(Style::default().bg(t.background).fg(t.foreground)),
        inner,
    );
}

fn draw_churn(f: &mut Frame, app: &App, area: Rect) {
    let t = &app.theme;
    let outer = centered(area, 40, 100, 10, 28);
    draw_solid_border(f, outer, t);

    let mut lines: Vec<Line<'static>> = vec![Line::from(Span::styled(
        format!(" churn — top {} (adds+dels, historial completo) ", app.churn_rows.len()),
        Style::default().fg(t.dimmed),
    ))];

    if app.churn_rows.is_empty() {
        lines.push(Line::from(Span::styled(
            " sin datos: r para escanear ",
            Style::default().fg(t.warning),
        )));
    } else {
        let max = app.churn_rows.iter().map(|(_, c, _)| *c).max().unwrap_or(1).max(1);
        let inner_w = outer.width.saturating_sub(4) as usize;
        // layout: [path 38][bar hasta 24][número]
        let path_w = 38.min(inner_w.saturating_sub(10));
        let bar_w = inner_w.saturating_sub(path_w + 8).clamp(4, 24);
        for (path, c, touches) in &app.churn_rows {
            let filled = ((*c as f32 / max as f32) * bar_w as f32).ceil() as usize;
            let bar = format!("{}{}", "█".repeat(filled), "░".repeat(bar_w - filled));
            lines.push(Line::from(vec![
                Span::styled(format!("{:<path_w$}", truncate(path, path_w)), Style::default().fg(t.foreground)),
                Span::styled(format!(" {bar} "), Style::default().fg(t.primary)),
                Span::styled(format!("{c:>5}"), Style::default().fg(t.accent)),
                Span::styled(format!(" ×{touches}"), Style::default().fg(t.dimmed)),
            ]));
        }
    }
    lines.push(Line::from(Span::styled(
        " Esc volver ",
        Style::default().fg(t.dimmed),
    )));

    let inner = Rect {
        x: outer.x + 2,
        y: outer.y + 1,
        width: outer.width.saturating_sub(4),
        height: outer.height.saturating_sub(2),
    };
    f.render_widget(
        Paragraph::new(lines).style(Style::default().bg(t.background).fg(t.foreground)),
        inner,
    );
}

fn truncate(s: &str, w: usize) -> String {
    if s.chars().count() <= w {
        s.to_string()
    } else if w > 3 {
        let head: String = s.chars().take(w - 3).collect();
        format!("{head}…")
    } else {
        s.chars().take(w).collect()
    }
}

/// Borde de bloque sólido (del fork, mismo patrón).
fn draw_solid_border(f: &mut Frame, area: Rect, t: &Theme) {
    let s = Style::default().fg(t.border).bg(t.border);
    let w = area.width as usize;
    let top = "\u{2588}".repeat(w);
    f.render_widget(Paragraph::new(top.clone()).style(s), Rect { x: area.x, y: area.y, width: area.width, height: 1 });
    f.render_widget(Paragraph::new(top).style(s), Rect { x: area.x, y: area.y + area.height - 1, width: area.width, height: 1 });
    for row in 1..area.height.saturating_sub(1) {
        let y = area.y + row;
        f.render_widget(Paragraph::new("\u{2588}").style(s), Rect { x: area.x, y, width: 1, height: 1 });
        f.render_widget(
            Paragraph::new("\u{2588}").style(s),
            Rect { x: area.x + area.width - 1, y, width: 1, height: 1 },
        );
    }
}
