// ── UI (presentación) ────────────────────────────────────────────────
// F2: dos vistas — resumen (F0/F1) y churn con barras de bloques.
// El consumo del engine es síncrono en `r` todavía; el worker + caché
// llega en F3. Nada de operaciones: corte de cordón (DECISIONS §2A).

use std::io::{self, Stdout};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::thread;
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
use gadv::engine::{
    self, FileId, History, Window, churn, head_locs, hotspots, is_ignored, ownership, repo_risk,
};
use gadv::log;
use gadv::theme::{self, Theme};
use gadv::version;

use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum View {
    Summary,
    Churn,
    Hotspot,
    Ownership,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimeWin {
    All,
    D90,
    D30,
}

impl TimeWin {
    fn next(self) -> Self {
        match self {
            TimeWin::All => TimeWin::D90,
            TimeWin::D90 => TimeWin::D30,
            TimeWin::D30 => TimeWin::All,
        }
    }
    fn label(self) -> &'static str {
        match self {
            TimeWin::All => "todo el historial",
            TimeWin::D90 => "ultimos 90 dias",
            TimeWin::D30 => "ultimos 30 dias",
        }
    }
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
    pub window: TimeWin,
    /// (path, churn, touches) top-20 ya ordenado, listo para pintar.
    pub churn_rows: Vec<(String, u32, u32)>,
    /// (path, churn, loc, score) top-20, para la vista Hotspot.
    pub hotspot_rows: Vec<(String, u32, u32, f32)>,
    /// (path, owner, share, bus_factor, kept) top-30 por riesgo, vista Ownership.
    pub ownership_rows: Vec<(String, String, f32, usize, u32)>,
    /// (módulos con bus factor 1, módulos con dueño) sobre los top por tamaño.
    pub repo_risk: (usize, usize),
    locs: HashMap<FileId, u32>,
    history: Option<History>,
    /// Worker de escaneo (F3): la UI nunca bloquea por el motor.
    rx: Option<Receiver<ScanResult>>,
    scanning: bool,
}

type ScanResult = Result<(History, HashMap<FileId, u32>, &'static str, u128), String>;

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
            window: TimeWin::All,
            churn_rows: Vec::new(),
            hotspot_rows: Vec::new(),
            ownership_rows: Vec::new(),
            repo_risk: (0, 0),
            locs: HashMap::new(),
            history: None,
            rx: None,
            scanning: false,
        }
    }

    /// Lee lo barato (nombre, HEAD) y lanza el escaneo pesado al worker.
    fn refresh(&mut self) {
        self.repo_name = engine::repo_name(&self.repo_path);
        self.head = engine::head_raw(&self.repo_path);
        self.start_scan();
    }

    fn start_scan(&mut self) {
        if self.scanning {
            return;
        }
        self.scanning = true;
        self.status = "escaneando…".into();
        let (tx, rx) = mpsc::channel();
        self.rx = Some(rx);
        let path = self.repo_path.clone();
        let max = self.max_commits;
        let use_cache = !self.no_cache;
        thread::spawn(move || {
            let t0 = Instant::now();
            let result = engine::scan_with_cache(&path, max, use_cache).map(|outcome| {
                let label = match outcome.source {
                    gadv::engine::ScanSource::Cache => "cache",
                    gadv::engine::ScanSource::Delta(_) => "delta",
                    gadv::engine::ScanSource::Full => "full",
                };
                // F4: LOC solo de los candidatos a hotspot (top-500 por churn).
                let history = outcome.history;
                let candidates: Vec<(FileId, &str)> = churn(&history, Window::ALL)
                    .into_iter()
                    .take(500)
                    .filter_map(|r| {
                        history
                            .paths
                            .get(r.file.0 as usize)
                            .map(|p| (r.file, p.as_str()))
                    })
                    .collect();
                let locs = head_locs(&path, &candidates).unwrap_or_default();
                (history, locs, label, t0.elapsed().as_millis())
            });
            let _ = tx.send(result.map_err(|e| e.to_string()));
        });
    }

    /// Drena el canal: un mensaje por frame basta (el worker manda uno).
    fn drain_scan(&mut self) {
        let Some(rx) = &self.rx else { return };
        match rx.try_recv() {
            Ok(Ok((history, locs, label, ms))) => {
                self.apply_history(history, locs, label, ms);
                self.scanning = false;
                self.rx = None;
            }
            Ok(Err(err)) => {
                self.commits = err;
                self.churn_rows.clear();
                self.hotspot_rows.clear();
                self.history = None;
                self.scanning = false;
                self.rx = None;
            }
            Err(TryRecvError::Disconnected) => {
                self.commits = "worker muerto".into();
                self.scanning = false;
                self.rx = None;
            }
            Err(TryRecvError::Empty) => {}
        }
    }

    fn apply_history(&mut self, history: History, locs: HashMap<FileId, u32>, label: &'static str, ms: u128) {
        self.commits = format!("{} commits en {ms} ms ({label})", history.commits.len());
        self.history = Some(history);
        self.locs = locs;
        self.recompute();
        self.scans += 1;
        self.status = format!("refrescado ×{}", self.scans);
    }

    /// Recalcula las filas de ambas vistas para la ventana actual.
    /// Puro y barato: la ventana filtra la caché, no re-ingesta (F4).
    fn recompute(&mut self) {
        let Some(history) = &self.history else { return };
        let window = match self.window {
            TimeWin::All => Window::ALL,
            days => {
                let now = history.commits.iter().map(|c| c.time).max().unwrap_or(0);
                let d = if days == TimeWin::D30 { 30 } else { 90 };
                Window {
                    from: Some(now - d * 86_400),
                }
            }
        };
        let path_of = |file: gadv::engine::FileId| {
            history
                .paths
                .get(file.0 as usize)
                .cloned()
                .unwrap_or_else(|| format!("#{}", file.0))
        };

        let churn_rows: Vec<(String, u32, u32)> = churn(history, window)
            .into_iter()
            .take(20)
            .map(|r| (path_of(r.file), r.churn(), r.touches))
            .collect();
        let locs = &self.locs;
        let hotspot_rows: Vec<(String, u32, u32, f32)> = hotspots(
            history,
            window,
            &|f| locs.get(&f).copied().unwrap_or(0),
            &|p| is_ignored(p, &[]),
            20,
        )
        .into_iter()
        .map(|r| (path_of(r.file), r.churn, r.loc, r.score))
        .collect();

        let own = ownership(history, window);
        let (bf1, total) = repo_risk(&own, 50);
        let ownership_rows: Vec<(String, String, f32, usize, u32)> = own
            .iter()
            .take(30)
            .filter(|r| r.bus_factor > 0)
            .map(|r| {
                let owner = r
                    .owner
                    .and_then(|a| history.authors.get(a.0 as usize))
                    .map(|a| a.name.clone())
                    .unwrap_or_else(|| "?".to_string());
                (path_of(r.file), owner, r.owner_share, r.bus_factor, r.kept_total)
            })
            .collect();

        self.churn_rows = churn_rows;
        self.hotspot_rows = hotspot_rows;
        self.ownership_rows = ownership_rows;
        self.repo_risk = (bf1, total);
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
        app.drain_scan();
        terminal.draw(|f| draw(f, app))?;
        if debug && first {
            log::log_debug("primer frame dibujado");
            first = false;
        }
        if event::poll(Duration::from_millis(50))?
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
                    View::Churn | View::Hotspot | View::Ownership => app.view = View::Summary,
                    View::Summary => return Ok(()),
                },
                KeyCode::Char('1') => app.view = View::Summary,
                KeyCode::Char('2') => {
                    if app.history.is_some() {
                        app.view = View::Churn;
                    }
                }
                KeyCode::Char('3') => {
                    if app.history.is_some() {
                        app.view = View::Hotspot;
                    }
                }
                KeyCode::Char('4') => {
                    if app.history.is_some() {
                        app.view = View::Ownership;
                    }
                }
                KeyCode::Char('t') => {
                    app.window = app.window.next();
                    app.recompute();
                }
                KeyCode::Char('r') => app.refresh(),
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
        View::Hotspot => draw_hotspot(f, app, area),
        View::Ownership => draw_ownership(f, app, area),
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
            "1 resumen · 2 churn · 3 hotspots · 4 ownership · t ventana · r refrescar · q/Esc salir",
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

/// Ownership por riesgo: bus factor 1 primero. El agregado del repo arriba.
fn draw_ownership(f: &mut Frame, app: &App, area: Rect) {
    let t = &app.theme;
    let outer = centered(area, 48, 100, 12, 30);
    draw_solid_border(f, outer, t);

    let mut lines: Vec<Line<'static>> = vec![Line::from(Span::styled(
        format!(" ownership — heurística por commits (no blame) · ventana: {} ", app.window.label()),
        Style::default().fg(t.dimmed),
    ))];
    let (bf1, total) = app.repo_risk;
    let risk_color = if bf1 > 0 { t.warning } else { t.success };
    lines.push(Line::from(Span::styled(
        format!(" módulos con bus factor 1: {bf1} de {total} "),
        Style::default().fg(risk_color).add_modifier(Modifier::BOLD),
    )));

    if app.ownership_rows.is_empty() {
        lines.push(Line::from(Span::styled(
            " sin datos: r para escanear ",
            Style::default().fg(t.dimmed),
        )));
    } else {
        for (path, owner, share, bf, kept) in &app.ownership_rows {
            let bar_w = 10usize;
            let filled = ((*share * bar_w as f32) as usize).clamp(1, bar_w);
            let bar = format!("{}{}", "█".repeat(filled), "░".repeat(bar_w - filled));
            let bf_color = if *bf <= 1 { t.warning } else { t.success };
            lines.push(Line::from(vec![
                Span::styled(format!("bf {bf} "), Style::default().fg(bf_color)),
                Span::styled(format!("{bar} "), Style::default().fg(t.primary)),
                Span::styled(format!("{:<14} ", truncate(owner, 14)), Style::default().fg(t.foreground)),
                Span::styled(format!("{kept:>6}  "), Style::default().fg(t.accent)),
                Span::styled(truncate(path, 38), Style::default().fg(t.dimmed)),
            ]));
        }
    }
    lines.push(Line::from(Span::styled(
        " t ventana · Esc volver ",
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

/// Scatter churn×LOC dibujado a mano (grid de celdas) + ranking top.
/// x = log2(LOC) para que el eje no lo domine un archivo de 50k líneas;
/// y = churn. Celda con N puntos muestra el de mayor score.
fn draw_hotspot(f: &mut Frame, app: &App, area: Rect) {
    let t = &app.theme;
    let outer = centered(area, 48, 110, 12, 30);
    draw_solid_border(f, outer, t);

    let mut lines: Vec<Line<'static>> = vec![Line::from(Span::styled(
        format!(
            " hotspots — churn × LOC (proxy) · ventana: {} ",
            app.window.label()
        ),
        Style::default().fg(t.dimmed),
    ))];

    if app.hotspot_rows.is_empty() {
        lines.push(Line::from(Span::styled(
            " sin candidatos: r para escanear ",
            Style::default().fg(t.warning),
        )));
    } else {
        // --- scatter grid ---
        let gw = 44usize;
        let gh = 10usize;
        let max_churn = app.hotspot_rows.iter().map(|(_, c, _, _)| *c).max().unwrap_or(1).max(1) as f32;
        let max_log = app
            .hotspot_rows
            .iter()
            .fold(1.0f32, |m, (_, _, l, _)| (m).max((*l as f32).log2()))
            .max(1.0);
        // celda -> índice del punto con mayor score en ella
        let mut grid: Vec<Vec<Option<usize>>> = vec![vec![None; gw]; gh];
        for (i, (_, c, l, _)) in app.hotspot_rows.iter().enumerate() {
            let x = (((*l as f32).log2() / max_log) * (gw - 1) as f32) as usize;
            let y = (( *c as f32 / max_churn) * (gh - 1) as f32) as usize;
            let cell = &mut grid[gh - 1 - y][x];
            if cell.is_none_or(|j| app.hotspot_rows[j].3 < app.hotspot_rows[i].3) {
                *cell = Some(i);
            }
        }
        for row in grid.iter() {
            let mut spans: Vec<Span<'static>> = Vec::with_capacity(gw);
            for cell in row {
                match cell {
                    Some(i) => {
                        let color = if *i < 3 { t.warning } else { t.accent };
                        spans.push(Span::styled("●", Style::default().fg(color)));
                    }
                    None => spans.push(Span::styled("·", Style::default().fg(t.border))),
                }
            }
            lines.push(Line::from(spans));
        }
        lines.push(Line::from(Span::styled(
            format!(
                " x: LOC 8→{} · y: churn 0→{} (los 3 rojos = mayor score) ",
                2u64.pow(max_log as u32),
                max_churn as u32
            ),
            Style::default().fg(t.dimmed),
        )));
        lines.push(Line::from(""));
        // --- ranking top-8 ---
        for (path, c, l, s) in app.hotspot_rows.iter().take(8) {
            lines.push(Line::from(vec![
                Span::styled(format!("{s:.2} "), Style::default().fg(t.warning)),
                Span::styled(format!("churn {c:<6} loc {l:<6} "), Style::default().fg(t.foreground)),
                Span::styled(truncate(path, 40), Style::default().fg(t.primary)),
            ]));
        }
    }
    lines.push(Line::from(Span::styled(
        " t ventana · Esc volver ",
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
