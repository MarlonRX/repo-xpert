// ── UI (presentación) ────────────────────────────────────────────────
// Layout a pantalla completa: cabecera con contexto, barra de pestañas,
// contenido por vista y pie con teclas. El engine nunca se toca acá:
// solo se consumen sus tipos. Nada de operaciones: corte de cordón.

use std::collections::HashMap;
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
    self, FileId, History, Window, churn, coupling, head_locs, hotspots, is_ignored, neighbors,
    ownership, repo_risk,
};
use gadv::log;
use gadv::theme::{self, Theme};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum View {
    Summary,
    Churn,
    Hotspot,
    Ownership,
    Coupling,
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
            TimeWin::All => "historial completo",
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
    pub cursor: usize,
    /// (file, path, churn, touches) top-20.
    pub churn_rows: Vec<(FileId, String, u32, u32)>,
    /// (file, path, churn, loc, score) top-20.
    pub hotspot_rows: Vec<(FileId, String, u32, u32, f32)>,
    /// (file, path, owner, share, bus_factor, kept) top-30 por riesgo.
    pub ownership_rows: Vec<(FileId, String, String, f32, usize, u32)>,
    /// (módulos bf1, módulos con dueño) sobre los top por tamaño.
    pub repo_risk: (usize, usize),
    coupling_edges: Vec<gadv::engine::CouplingEdge>,
    pub coupling_rows: Vec<(String, f32, u32)>,
    pub coupling_title: String,
    locs: HashMap<FileId, u32>,
    history: Option<History>,
    rx: Option<Receiver<ScanResult>>,
    scanning: bool,
    /// total de commits y merges para el resumen (se guarda al aplicar).
    totals: (usize, usize, usize),
}

type ScanResult = Result<(History, HashMap<FileId, u32>, &'static str, u128), String>;

/// 4,080 en vez de 4080: los números de métricas se leen de un vistazo.
fn fmt(n: u32) -> String {
    let s = n.to_string();
    let mut out = String::with_capacity(s.len() + s.len() / 3);
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
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
            window: TimeWin::All,
            cursor: 0,
            churn_rows: Vec::new(),
            hotspot_rows: Vec::new(),
            ownership_rows: Vec::new(),
            repo_risk: (0, 0),
            coupling_edges: Vec::new(),
            coupling_rows: Vec::new(),
            coupling_title: String::new(),
            locs: HashMap::new(),
            history: None,
            rx: None,
            scanning: false,
            totals: (0, 0, 0),
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
                self.ownership_rows.clear();
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
        self.totals = (history.commits.len(), history.n_merges(), history.authors.len());
        self.history = Some(history);
        self.locs = locs;
        self.recompute();
        self.scans += 1;
        self.status = format!("listo · {label}");
    }

    /// Recalcula las filas de todas las vistas para la ventana actual.
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
        let path_of = |file: FileId| {
            history
                .paths
                .get(file.0 as usize)
                .cloned()
                .unwrap_or_else(|| format!("#{}", file.0))
        };

        let churn_rows: Vec<(FileId, String, u32, u32)> = churn(history, window)
            .into_iter()
            .take(20)
            .map(|r| (r.file, path_of(r.file), r.churn(), r.touches))
            .collect();
        let locs = &self.locs;
        let hotspot_rows: Vec<(FileId, String, u32, u32, f32)> = hotspots(
            history,
            window,
            &|f| locs.get(&f).copied().unwrap_or(0),
            &|p| is_ignored(p, &[]),
            20,
        )
        .into_iter()
        .map(|r| (r.file, path_of(r.file), r.churn, r.loc, r.score))
        .collect();

        let own = ownership(history, window);
        let (bf1, total) = repo_risk(&own, 50);
        let ownership_rows: Vec<(FileId, String, String, f32, usize, u32)> = own
            .iter()
            .take(30)
            .filter(|r| r.bus_factor > 0)
            .map(|r| {
                let owner = r
                    .owner
                    .and_then(|a| history.authors.get(a.0 as usize))
                    .map(|a| a.name.clone())
                    .unwrap_or_else(|| "?".to_string());
                (r.file, path_of(r.file), owner, r.owner_share, r.bus_factor, r.kept_total)
            })
            .collect();
        let coupling_edges = coupling(history, window, &|p| is_ignored(p, &[]));

        self.churn_rows = churn_rows;
        self.hotspot_rows = hotspot_rows;
        self.ownership_rows = ownership_rows;
        self.repo_risk = (bf1, total);
        self.coupling_edges = coupling_edges;
        self.cursor = 0;
    }

    fn open_coupling(&mut self) {
        let sel = match self.view {
            View::Churn => self.churn_rows.get(self.cursor).map(|r| (r.0, r.1.clone())),
            View::Hotspot => self.hotspot_rows.get(self.cursor).map(|r| (r.0, r.1.clone())),
            View::Ownership => self.ownership_rows.get(self.cursor).map(|r| (r.0, r.1.clone())),
            _ => None,
        };
        let Some((file, title)) = sel else { return };
        let rows: Vec<(String, f32, u32)> = neighbors(&self.coupling_edges, file, 5)
            .into_iter()
            .map(|(f, j, c)| {
                let path = self
                    .history
                    .as_ref()
                    .and_then(|h| h.paths.get(f.0 as usize).cloned())
                    .unwrap_or_else(|| format!("#{}", f.0));
                (path, j, c)
            })
            .collect();
        self.coupling_rows = rows;
        self.coupling_title = title;
        self.view = View::Coupling;
    }

    fn list_len(&self) -> usize {
        match self.view {
            View::Churn => self.churn_rows.len(),
            View::Hotspot => self.hotspot_rows.len(),
            View::Ownership => self.ownership_rows.len(),
            _ => 0,
        }
    }
}

// ── ciclo de vida ─────────────────────────────────────────────────────

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
                    View::Coupling => app.view = View::Churn,
                    View::Churn | View::Hotspot | View::Ownership => app.view = View::Summary,
                    View::Summary => return Ok(()),
                },
                KeyCode::Char('j') => {
                    let n = app.list_len();
                    if n > 0 {
                        app.cursor = (app.cursor + 1).min(n - 1);
                    }
                }
                KeyCode::Char('k') => {
                    app.cursor = app.cursor.saturating_sub(1);
                }
                KeyCode::Down => {
                    let n = app.list_len();
                    if n > 0 {
                        app.cursor = (app.cursor + 1).min(n - 1);
                    }
                }
                KeyCode::Up => {
                    app.cursor = app.cursor.saturating_sub(1);
                }
                KeyCode::Enter => app.open_coupling(),
                KeyCode::Char('1') => app.view = View::Summary,
                KeyCode::Char('2') if app.history.is_some() => app.view = View::Churn,
                KeyCode::Char('3') if app.history.is_some() => app.view = View::Hotspot,
                KeyCode::Char('4') if app.history.is_some() => app.view = View::Ownership,
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

// ── layout ────────────────────────────────────────────────────────────

fn draw(f: &mut Frame, app: &App) {
    let a = f.area();
    let t = &app.theme;
    f.render_widget(Paragraph::new("").style(Style::default().bg(t.background)), a);
    if a.width < 48 || a.height < 12 {
        let tiny = Paragraph::new("terminal muy chica: agranda a 48x12+")
            .style(Style::default().fg(t.warning).bg(t.background));
        f.render_widget(tiny, a);
        return;
    }

    let w = a.width;
    let x = a.x;
    draw_header(f, app, Rect { x, y: a.y, width: w, height: 2 });
    draw_tabs(f, app, Rect { x, y: a.y + 2, width: w, height: 1 });
    draw_rule(f, t, Rect { x, y: a.y + 3, width: w, height: 1 });
    let content = Rect {
        x,
        y: a.y + 4,
        width: w,
        height: a.height.saturating_sub(6),
    };
    match app.view {
        View::Summary => draw_summary(f, app, content),
        View::Churn => draw_churn(f, app, content),
        View::Hotspot => draw_hotspot(f, app, content),
        View::Ownership => draw_ownership(f, app, content),
        View::Coupling => draw_coupling(f, app, content),
    }
    draw_footer(f, app, Rect { x, y: a.y + a.height - 2, width: w, height: 1 });
}

fn draw_header(f: &mut Frame, app: &App, area: Rect) {
    let t = &app.theme;
    let line = Line::from(vec![
        Span::styled(
            " repo-xpert ",
            Style::default()
                .fg(t.on_highlight)
                .bg(t.primary)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled("  ", Style::default()),
        Span::styled(app.repo_name.clone(), Style::default().fg(t.foreground).add_modifier(Modifier::BOLD)),
        Span::styled("   ", Style::default()),
        Span::styled("HEAD", Style::default().fg(t.dimmed)),
        Span::styled(" ", Style::default()),
        Span::styled(app.head.clone(), Style::default().fg(t.success)),
        Span::styled("   ", Style::default()),
        Span::styled("ventana", Style::default().fg(t.dimmed)),
        Span::styled(" ", Style::default()),
        Span::styled(format!("[{}]", app.window.label()), Style::default().fg(t.accent)),
    ]);
    f.render_widget(Paragraph::new(line).style(Style::default().bg(t.background)), area);
}

fn draw_tabs(f: &mut Frame, app: &App, area: Rect) {
    let t = &app.theme;
    let tabs = [
        ("1", "Resumen", View::Summary),
        ("2", "Churn", View::Churn),
        ("3", "Hotspots", View::Hotspot),
        ("4", "Dueño", View::Ownership),
    ];
    let mut spans = vec![Span::styled(" ", Style::default())];
    for (key, label, view) in tabs {
        let active = app.view == view;
        let text = format!(" {key} {label} ");
        spans.push(Span::styled(
            text.clone(),
            if active {
                Style::default()
                    .fg(t.on_highlight)
                    .bg(t.surface)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(t.dimmed).bg(t.background)
            },
        ));
        spans.push(Span::styled("  ", Style::default()));
    }
    if app.scanning {
        spans.push(Span::styled("escaneando…", Style::default().fg(t.warning)));
    }
    f.render_widget(Paragraph::new(Line::from(spans)).style(Style::default().bg(t.background)), area);
}

fn draw_rule(f: &mut Frame, t: &Theme, area: Rect) {
    f.render_widget(
        Paragraph::new("─".repeat(area.width as usize)).style(Style::default().fg(t.border)),
        area,
    );
}

fn draw_footer(f: &mut Frame, app: &App, area: Rect) {
    let t = &app.theme;
    let keys = match app.view {
        View::Summary => "2/3/4 ir a metricas · r rescanear · t ventana · q salir",
        View::Coupling => "Esc volver · q salir",
        _ => "j/k o ↑/↓ seleccionar · Enter vecinos · Esc volver · t ventana · r rescanear · q salir",
    };
    let mut spans = vec![Span::styled(keys, Style::default().fg(t.dimmed))];
    if !app.commits.is_empty() {
        spans.push(Span::styled(
            format!("   {}", app.commits),
            Style::default().fg(t.border),
        ));
    }
    f.render_widget(Paragraph::new(Line::from(spans)).style(Style::default().bg(t.background)), area);
}

/// Encabezado de sección reutilizable.
fn section(title: &str, hint: &str, t: &Theme) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!(" {title}"), Style::default().fg(t.primary).add_modifier(Modifier::BOLD)),
        Span::styled(format!("   {hint}"), Style::default().fg(t.dimmed)),
    ])
}

fn empty_state(msg: &str, t: &Theme) -> Vec<Line<'static>> {
    vec![
        Line::from(""),
        Line::from(Span::styled(format!("   {msg}"), Style::default().fg(t.warning))),
    ]
}

// ── vistas ────────────────────────────────────────────────────────────

fn draw_summary(f: &mut Frame, app: &App, area: Rect) {
    let t = &app.theme;
    if app.history.is_none() {
        let msg = if app.scanning {
            "escaneando el historial… los repos grandes tardan unos segundos"
        } else {
            "sin datos: presiona r para escanear"
        };
        let mut lines = empty_state(msg, t);
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            " repo-xpert analiza tu repo localmente: que cambia, quien lo conoce, donde duele.",
            Style::default().fg(t.dimmed),
        )));
        f.render_widget(Paragraph::new(lines).style(Style::default().bg(t.background)), area);
        return;
    }

    let (ncommits, nmerges, nauthors) = app.totals;
    let kpi = |label: &'static str, value: String, color: ratatui::style::Color| {
        vec![
            Line::from(Span::styled(format!(" {label}"), Style::default().fg(t.dimmed))),
            Line::from(Span::styled(format!(" {value}"), Style::default().fg(color).add_modifier(Modifier::BOLD))),
        ]
    };

    let mut lines: Vec<Line<'static>> = Vec::new();
    lines.push(section("resumen", "una mirada al estado del repo", t));
    lines.push(Line::from(""));

    let top_churn = app.churn_rows.first().map(|(_, p, c, _)| format!("{p} · {}", fmt(*c)));
    let top_hot = app
        .hotspot_rows
        .first()
        .map(|(_, p, _, _, s)| format!("{p} · score {s:.2}"));
    let (bf1, total) = app.repo_risk;
    let risk = format!("{bf1} de {total} modulos con un solo dueno");

    let row1 = Line::from(vec![
        Span::styled(format!("{:<24}", fmt(ncommits as u32)), Style::default().fg(t.foreground).add_modifier(Modifier::BOLD)),
        Span::styled(format!("{:<24}", nauthors.to_string()), Style::default().fg(t.foreground).add_modifier(Modifier::BOLD)),
        Span::styled(nmerges.to_string(), Style::default().fg(t.foreground).add_modifier(Modifier::BOLD)),
    ]);
    lines.push(Line::from(vec![
        Span::styled(format!("{:<24}", " COMMITS"), Style::default().fg(t.dimmed)),
        Span::styled(format!("{:<24}", " AUTORES"), Style::default().fg(t.dimmed)),
        Span::styled(" MERGES", Style::default().fg(t.dimmed)),
    ]));
    lines.push(row1);
    lines.push(Line::from(""));

    lines.extend(kpi("MAYOR CHURN", top_churn.unwrap_or_else(|| "—".into()), t.primary));
    lines.push(Line::from(""));
    lines.extend(kpi("HOTSPOT #1", top_hot.unwrap_or_else(|| "—".into()), t.accent));
    lines.push(Line::from(""));
    lines.extend(kpi(
        "RIESGO DE CONOCIMIENTO",
        risk,
        if bf1 > 0 { t.warning } else { t.success },
    ));
    lines.push(Line::from(""));

    // mini barras top-5 churn para dar contexto visual inmediato
    let max = app.churn_rows.iter().map(|(_, _, c, _)| *c).max().unwrap_or(1).max(1);
    lines.push(Line::from(Span::styled(" top churn:", Style::default().fg(t.dimmed))));
    for (_, path, c, _) in app.churn_rows.iter().take(5) {
        let bar_w = (area.width.saturating_sub(50) as usize).clamp(8, 36);
        let filled = ((*c as f32 / max as f32) * bar_w as f32).ceil() as usize;
        lines.push(Line::from(vec![
            Span::styled(format!(" {} ", "▍".repeat(filled.max(1))), Style::default().fg(t.primary)),
            Span::styled(format!("{:>8}  ", fmt(*c)), Style::default().fg(t.foreground)),
            Span::styled(truncate(path, 42), Style::default().fg(t.dimmed)),
        ]));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "  2 churn · 3 hotspots · 4 dueño del código — Enter en una fila muestra que archivos cambian juntos",
        Style::default().fg(t.dimmed),
    )));

    f.render_widget(Paragraph::new(lines).style(Style::default().bg(t.background)), area);
}

fn draw_churn(f: &mut Frame, app: &App, area: Rect) {
    let t = &app.theme;
    if app.churn_rows.is_empty() {
        let lines = empty_state("nada en esta ventana — prueba t para cambiarla", t);
        f.render_widget(Paragraph::new(lines).style(Style::default().bg(t.background)), area);
        return;
    }
    let mut lines = vec![
        section("churn", "lineas agregadas + borradas por archivo (todo el historial = inestabilidad)", t),
        Line::from(Span::styled(
            "  #   churn   +adds   −dels    ×   archivo",
            Style::default().fg(t.dimmed),
        )),
    ];
    let max = app.churn_rows.iter().map(|(_, _, c, _)| *c).max().unwrap_or(1).max(1);
    let bar_w = (area.width.saturating_sub(46) as usize).clamp(6, 28);
    // suma total para el share relativo
    for (i, (_, path, c, touches)) in app.churn_rows.iter().enumerate() {
        let filled = ((*c as f32 / max as f32) * bar_w as f32).ceil() as usize;
        let sel = i == app.cursor;
        let row_bg = if sel { t.surface } else { t.background };
        let cur = if sel { "▸" } else { " " };
        lines.push(Line::from(vec![
            Span::styled(cur.to_string(), Style::default().fg(t.primary).bg(row_bg)),
            Span::styled(format!("{:>2} ", i + 1), Style::default().fg(t.dimmed).bg(row_bg)),
            Span::styled(
                format!("{}{} ", "▍".repeat(filled.max(1)), " ".repeat(bar_w - filled)),
                Style::default().fg(t.primary).bg(row_bg),
            ),
            Span::styled(format!("{:>8} ", fmt(*c)), Style::default().fg(t.foreground).bg(row_bg)),
            Span::styled(format!("×{touches:<3}"), Style::default().fg(t.dimmed).bg(row_bg)),
            Span::styled(" ", Style::default().bg(row_bg)),
            Span::styled(truncate(path, 46), Style::default().fg(if sel { t.highlight } else { t.accent }).bg(row_bg)),
        ]));
    }
    f.render_widget(Paragraph::new(lines).style(Style::default().bg(t.background)), area);
}

fn draw_hotspot(f: &mut Frame, app: &App, area: Rect) {
    let t = &app.theme;
    if app.hotspot_rows.is_empty() {
        let lines = empty_state("sin candidatos en esta ventana", t);
        f.render_widget(Paragraph::new(lines).style(Style::default().bg(t.background)), area);
        return;
    }
    let mut lines = vec![section(
        "hotspots",
        "arriba-derecha = cambia mucho Y es grande: ahi viven los bugs",
        t,
    )];

    // scatter
    let gw = (area.width.saturating_sub(12) as usize).clamp(24, 56);
    let gh = (area.height.saturating_sub(14) as usize).clamp(6, 12);
    let max_churn = app.hotspot_rows.iter().map(|(_, _, c, _, _)| *c).max().unwrap_or(1).max(1) as f32;
    let max_log = app
        .hotspot_rows
        .iter()
        .fold(1.0f32, |m, (_, _, _, l, _)| m.max((*l as f32).log2()))
        .max(1.0);
    let mut grid: Vec<Vec<Option<usize>>> = vec![vec![None; gw]; gh];
    for (i, (_, _, c, l, _)) in app.hotspot_rows.iter().enumerate() {
        let x = (((*l as f32).log2() / max_log) * (gw - 1) as f32) as usize;
        let y = ((*c as f32 / max_churn) * (gh - 1) as f32) as usize;
        let cell = &mut grid[gh - 1 - y][x];
        if cell.is_none_or(|j| app.hotspot_rows[j].4 < app.hotspot_rows[i].4) {
            *cell = Some(i);
        }
    }
    for (r, row) in grid.iter().enumerate() {
        let mut spans = vec![Span::styled(" ", Style::default())];
        for cell in row {
            match cell {
                Some(i) => {
                    let color = if *i == app.cursor {
                        t.highlight
                    } else if *i < 3 {
                        t.warning
                    } else {
                        t.primary
                    };
                    spans.push(Span::styled("●", Style::default().fg(color)));
                }
                None => spans.push(Span::styled("·", Style::default().fg(t.border))),
            }
        }
        let label = if r == 0 { "churn alto " } else if r == gh - 1 { "churn bajo " } else { "          " };
        spans.push(Span::styled(format!(" {label}"), Style::default().fg(t.dimmed)));
        lines.push(Line::from(spans));
    }
    lines.push(Line::from(Span::styled(
        format!(
            "            LOC: 8 → {} (escala log)      ● top 3 en coral · resaltado = seleccionado",
            2u64.pow(max_log as u32)
        ),
        Style::default().fg(t.dimmed),
    )));
    lines.push(Line::from(""));

    // ranking
    lines.push(Line::from(Span::styled(
        "  #  score   churn      LOC  archivo",
        Style::default().fg(t.dimmed),
    )));
    for (i, (_, path, c, l, s)) in app.hotspot_rows.iter().take(6).enumerate() {
        let sel = i == app.cursor;
        lines.push(Line::from(vec![
            Span::styled(if sel { "▸" } else { " " }, Style::default().fg(t.primary)),
            Span::styled(format!("{:>2} ", i + 1), Style::default().fg(t.dimmed)),
            Span::styled(format!("{s:.2}   "), Style::default().fg(t.warning)),
            Span::styled(format!("{:>8} ", fmt(*c)), Style::default().fg(t.foreground)),
            Span::styled(format!("{:>8} ", fmt(*l)), Style::default().fg(t.foreground)),
            Span::styled(truncate(path, 42), Style::default().fg(if sel { t.highlight } else { t.accent })),
        ]));
    }
    f.render_widget(Paragraph::new(lines).style(Style::default().bg(t.background)), area);
}

fn draw_ownership(f: &mut Frame, app: &App, area: Rect) {
    let t = &app.theme;
    let (bf1, total) = app.repo_risk;
    let mut lines = vec![section(
        "dueño del código",
        "quien escribe y nadie mas puede mantener (heurística por commits, no blame)",
        t,
    )];
    lines.push(Line::from(vec![
        Span::styled("  riesgo de bus factor: ", Style::default().fg(t.dimmed)),
        Span::styled(
            format!("{bf1} de {total}"),
            Style::default()
                .fg(if bf1 > 0 { t.warning } else { t.success })
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            if bf1 > 0 { " modulos con un solo dueño" } else { " todo el codigo tiene respaldo" },
            Style::default().fg(if bf1 > 0 { t.warning } else { t.success }),
        ),
    ]));
    lines.push(Line::from(""));
    if app.ownership_rows.is_empty() {
        lines.extend(empty_state("sin datos en esta ventana", t));
        f.render_widget(Paragraph::new(lines).style(Style::default().bg(t.background)), area);
        return;
    }
    lines.push(Line::from(Span::styled(
        "      dueño   share          líneas que nadie mas conoce   archivo",
        Style::default().fg(t.dimmed),
    )));
    for (i, (_, path, owner, share, bf, kept)) in app.ownership_rows.iter().take(12).enumerate() {
        let bar_w = 10usize;
        let filled = ((*share * bar_w as f32) as usize).clamp(1, bar_w);
        let sel = i == app.cursor;
        let bf_color = if *bf <= 1 { t.warning } else { t.success };
        lines.push(Line::from(vec![
            Span::styled(if sel { "▸" } else { " " }, Style::default().fg(t.primary)),
            Span::styled(format!("bf {bf} "), Style::default().fg(bf_color)),
            Span::styled(format!("{:<12} ", truncate(owner, 12)), Style::default().fg(t.foreground)),
            Span::styled(
                format!("{}{} ", "▍".repeat(filled), "·".repeat(bar_w - filled)),
                Style::default().fg(t.primary),
            ),
            Span::styled(format!("{:>8}  ", fmt(*kept)), Style::default().fg(t.accent)),
            Span::styled(truncate(path, 44), Style::default().fg(if sel { t.highlight } else { t.dimmed })),
        ]));
    }
    f.render_widget(Paragraph::new(lines).style(Style::default().bg(t.background)), area);
}

fn draw_coupling(f: &mut Frame, app: &App, area: Rect) {
    let t = &app.theme;
    let mut lines = vec![section(
        &format!("cambia junto con · {}", truncate(&app.coupling_title, 44)),
        "archivos que se tocan en los mismos commits (dependencias ocultas)",
        t,
    )];
    lines.push(Line::from(""));
    if app.coupling_rows.is_empty() {
        lines.extend(empty_state(
            "nadie cambia junto a este archivo (minimo 3 co-commits)",
            t,
        ));
    }
    for (path, j, cooc) in &app.coupling_rows {
        let bar_w = 14usize;
        let filled = ((*j * bar_w as f32) as usize).clamp(1, bar_w);
        lines.push(Line::from(vec![
            Span::styled(
                format!(" {}{} ", "▍".repeat(filled), "·".repeat(bar_w - filled)),
                Style::default().fg(t.accent),
            ),
            Span::styled(format!("{j:.2}"), Style::default().fg(t.foreground)),
            Span::styled(format!(" ×{cooc:<3} "), Style::default().fg(t.dimmed)),
            Span::styled(truncate(path, 52), Style::default().fg(t.primary)),
        ]));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "  1.00 = siempre juntos · Esc para volver",
        Style::default().fg(t.dimmed),
    )));
    f.render_widget(Paragraph::new(lines).style(Style::default().bg(t.background)), area);
}

// ── helpers ───────────────────────────────────────────────────────────

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fmt_adds_thousands_separators() {
        assert_eq!(fmt(0), "0");
        assert_eq!(fmt(999), "999");
        assert_eq!(fmt(1000), "1,000");
        assert_eq!(fmt(4080), "4,080");
        assert_eq!(fmt(1000000), "1,000,000");
    }
}
