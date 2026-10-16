use std::{
    collections::{HashSet, VecDeque},
    io::{self, IsTerminal, Write},
    sync::{
        Arc, LazyLock, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind, KeyModifiers},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use pumpkin_config::AdvancedConfiguration;
use ratatui::{
    Frame, Terminal,
    backend::CrosstermBackend,
    layout::{Constraint, Layout},
    style::{Color, Style},
    text::Line,
    widgets::{Block, List, ListItem, Paragraph},
};
use rustyline::{completion::Completer, history::DefaultHistory};
use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System};

use crate::{STOP_INTERRUPT, server::Server};

const MAX_LOG_LINES: usize = 1000;
const MAX_LINE_CHARS: usize = 4096;
const MAX_HISTORY: usize = 100;
const REFRESH_INTERVAL: Duration = Duration::from_secs(1);
const INPUT_INTERVAL: Duration = Duration::from_millis(100);

static ACTIVE: AtomicBool = AtomicBool::new(false);
static TERMINAL_OPEN: AtomicBool = AtomicBool::new(false);
static LOGS: LazyLock<Mutex<VecDeque<String>>> = LazyLock::new(|| Mutex::new(VecDeque::new()));

pub(crate) struct ConsolePrinter;

impl rustyline::ExternalPrinter for ConsolePrinter {
    fn print(&mut self, message: String) -> rustyline::Result<()> {
        if is_active() {
            push_log(&crate::logging::remove_ansi_color_code(&message));
        } else {
            let mut stdout = io::stdout().lock();
            stdout.write_all(message.as_bytes())?;
            stdout.flush()?;
        }
        Ok(())
    }
}

pub(crate) fn prepare(config: &AdvancedConfiguration) {
    ACTIVE.store(
        config.tui.enabled && io::stdin().is_terminal() && io::stdout().is_terminal(),
        Ordering::Release,
    );
}

pub(crate) fn is_active() -> bool {
    ACTIVE.load(Ordering::Acquire)
}

pub(crate) fn push_log(message: &str) {
    let mut logs = LOGS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    for line in message.lines() {
        if logs.len() == MAX_LOG_LINES {
            logs.pop_front();
        }
        logs.push_back(
            line.chars()
                .filter(|c| !c.is_control())
                .take(MAX_LINE_CHARS)
                .collect(),
        );
    }
}

/// Restores the terminal before reporting a panic.
pub fn restore_terminal() {
    if TERMINAL_OPEN.swap(false, Ordering::AcqRel) {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen, crossterm::cursor::Show);
    }
    ACTIVE.store(false, Ordering::Release);
}

pub(crate) struct Dashboard {
    running: Arc<AtomicBool>,
    task: tokio::task::JoinHandle<()>,
}

impl Dashboard {
    pub(crate) async fn finish(self) {
        self.running.store(false, Ordering::Release);
        if let Err(err) = self.task.await {
            restore_terminal();
            tracing::error!("Terminal dashboard task failed: {err}");
        }
    }
}

struct TerminalGuard;

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        restore_terminal();
    }
}

pub(crate) fn start(server: &Arc<Server>) -> Option<Dashboard> {
    if !is_active() {
        return None;
    }
    let terminal = (|| {
        enable_raw_mode()?;
        TERMINAL_OPEN.store(true, Ordering::Release);
        execute!(io::stdout(), EnterAlternateScreen)?;
        Terminal::new(CrosstermBackend::new(io::stdout()))
    })();
    let terminal = match terminal {
        Ok(terminal) => terminal,
        Err(err) => {
            restore_terminal();
            tracing::warn!("Failed to initialize terminal dashboard: {err}");
            return None;
        }
    };
    let (tx, mut rx) = tokio::sync::mpsc::channel::<String>(16);
    let command_server = server.clone();
    server.spawn_task(async move {
        loop {
            let line = tokio::select! {
                line = rx.recv() => line,
                () = STOP_INTERRUPT.cancelled() => None,
            };
            let Some(line) = line else { break };
            crate::dispatch_console_command(&command_server, &line).await;
        }
    });
    let running = Arc::new(AtomicBool::new(true));
    let task_running = running.clone();
    let server = server.clone();
    let task = tokio::task::spawn_blocking(move || {
        let guard = TerminalGuard;
        let result = run(terminal, &server, &tx, &task_running);
        drop(guard);
        if let Err(err) = result {
            tracing::error!("Terminal dashboard failed: {err}");
            if server.advanced_config.commands.use_console {
                crate::setup_stdin_console(&server);
            }
        }
    });
    Some(Dashboard { running, task })
}

#[derive(Default)]
struct View {
    players: Vec<(String, bool)>,
    cpu: Option<f32>,
    memory: Option<u64>,
    input: Vec<char>,
    cursor: usize,
    log_scroll: usize,
    player_scroll: usize,
    history: VecDeque<String>,
    history_index: Option<usize>,
    draft: Vec<char>,
}

impl View {
    fn refresh(&mut self, server: &Server, system: &mut System) {
        if let Ok(pid) = sysinfo::get_current_pid() {
            system.refresh_processes_specifics(
                ProcessesToUpdate::Some(&[pid]),
                true,
                ProcessRefreshKind::nothing().with_cpu().with_memory(),
            );
            if let Some(process) = system.process(pid) {
                self.cpu = Some(process.cpu_usage());
                self.memory = Some(process.memory());
            }
        }
        let operators: HashSet<_> = server
            .data
            .operator_config
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .ops
            .iter()
            .map(|op| op.uuid)
            .collect();
        self.players = server
            .get_all_players()
            .iter()
            .map(|player| {
                (
                    player.gameprofile.name.clone(),
                    operators.contains(&player.gameprofile.id),
                )
            })
            .collect();
        self.players.sort_unstable_by(|a, b| a.0.cmp(&b.0));
        self.player_scroll = self.player_scroll.min(self.players.len().saturating_sub(1));
    }

    fn history(&mut self, previous: bool) {
        if self.history.is_empty() || (!previous && self.history_index.is_none()) {
            return;
        }
        if previous {
            if self.history_index.is_none() {
                self.draft.clone_from(&self.input);
            }
            self.history_index = Some(
                self.history_index
                    .map_or(self.history.len() - 1, |i| i.saturating_sub(1)),
            );
        } else {
            self.history_index = self
                .history_index
                .and_then(|i| (i + 1 < self.history.len()).then_some(i + 1));
        }
        self.input = self
            .history_index
            .map_or_else(|| self.draft.clone(), |i| self.history[i].chars().collect());
        self.cursor = self.input.len();
    }
}

fn run(
    mut terminal: Terminal<CrosstermBackend<io::Stdout>>,
    server: &Arc<Server>,
    tx: &tokio::sync::mpsc::Sender<String>,
    running: &AtomicBool,
) -> io::Result<()> {
    let mut view = View::default();
    let mut system = System::new();
    let completer = crate::logging::PumpkinCommandCompleter::new();
    if let Ok(mut slot) = completer.server.write() {
        *slot = Some(server.clone());
    }
    view.refresh(server, &mut system);
    let mut refreshed = Instant::now();
    while running.load(Ordering::Acquire) && is_active() {
        if refreshed.elapsed() >= REFRESH_INTERVAL {
            view.refresh(server, &mut system);
            refreshed = Instant::now();
        }
        terminal.draw(|frame| draw(frame, &view, server.advanced_config.commands.use_console))?;
        if !event::poll(INPUT_INTERVAL)? {
            continue;
        }
        let Event::Key(key) = event::read()? else {
            continue;
        };
        if key.kind == KeyEventKind::Release {
            continue;
        }
        if key.modifiers.contains(KeyModifiers::CONTROL)
            && matches!(key.code, KeyCode::Char('c' | 'd'))
        {
            crate::stop_or_exit_server();
            continue;
        }
        handle_key(
            &mut view,
            key,
            tx,
            &completer,
            server.advanced_config.commands.use_console,
        );
    }
    Ok(())
}

fn handle_key(
    view: &mut View,
    key: crossterm::event::KeyEvent,
    tx: &tokio::sync::mpsc::Sender<String>,
    completer: &crate::logging::PumpkinCommandCompleter,
    console_enabled: bool,
) {
    match key.code {
        KeyCode::PageUp => view.log_scroll = (view.log_scroll + 10).min(MAX_LOG_LINES - 1),
        KeyCode::PageDown => view.log_scroll = view.log_scroll.saturating_sub(10),
        KeyCode::Up if key.modifiers.contains(KeyModifiers::ALT) => {
            view.player_scroll = view.player_scroll.saturating_sub(1);
        }
        KeyCode::Down if key.modifiers.contains(KeyModifiers::ALT) => {
            view.player_scroll = (view.player_scroll + 1).min(view.players.len().saturating_sub(1));
        }
        _ if !console_enabled => {}
        KeyCode::Char(c)
            if !key
                .modifiers
                .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
                && view.input.len() < MAX_LINE_CHARS =>
        {
            view.input.insert(view.cursor, c);
            view.cursor += 1;
        }
        KeyCode::Backspace if view.cursor > 0 => {
            view.cursor -= 1;
            view.input.remove(view.cursor);
        }
        KeyCode::Delete if view.cursor < view.input.len() => {
            view.input.remove(view.cursor);
        }
        KeyCode::Left => view.cursor = view.cursor.saturating_sub(1),
        KeyCode::Right => view.cursor = (view.cursor + 1).min(view.input.len()),
        KeyCode::Home => view.cursor = 0,
        KeyCode::End => view.cursor = view.input.len(),
        KeyCode::Up => view.history(true),
        KeyCode::Down => view.history(false),
        KeyCode::Tab => {
            let line: String = view.input.iter().collect();
            let prefix: String = view.input[..view.cursor].iter().collect();
            let history = DefaultHistory::new();
            let context = rustyline::Context::new(&history);
            if let Ok((start, candidates)) = completer.complete(&line, prefix.len(), &context) {
                if let [candidate] = candidates.as_slice() {
                    let completed =
                        format!("{}{candidate}{}", &line[..start], &line[prefix.len()..]);
                    if completed.chars().count() <= MAX_LINE_CHARS {
                        view.cursor = line[..start].chars().count() + candidate.chars().count();
                        view.input = completed.chars().collect();
                    }
                } else if !candidates.is_empty() {
                    push_log(&candidates.join("  "));
                    view.log_scroll = 0;
                }
            }
        }
        KeyCode::Enter => {
            let line: String = view.input.iter().collect();
            let line = line.trim();
            if !line.is_empty() {
                match tx.try_send(line.to_string()) {
                    Ok(()) => {
                        push_log(&format!("> {line}"));
                        if view.history.len() == MAX_HISTORY {
                            view.history.pop_front();
                        }
                        view.history.push_back(line.to_string());
                        view.input.clear();
                        view.cursor = 0;
                        view.history_index = None;
                        view.draft.clear();
                        view.log_scroll = 0;
                    }
                    Err(err) => tracing::warn!("Unable to submit console command: {err}"),
                }
            }
        }
        _ => {}
    }
}

fn draw(frame: &mut Frame<'_>, view: &View, console_enabled: bool) {
    let [stats, body, input, help] = Layout::vertical([
        Constraint::Length(3),
        Constraint::Min(0),
        Constraint::Length(3),
        Constraint::Length(1),
    ])
    .areas(frame.area());
    let [console, players] =
        Layout::horizontal([Constraint::Percentage(75), Constraint::Percentage(25)]).areas(body);
    let cpu = view
        .cpu
        .map_or_else(|| "N/A".to_string(), |cpu| format!("{cpu:.1}%"));
    let memory = view.memory.map_or_else(
        || "N/A".to_string(),
        |bytes| format!("{:.1} MiB", bytes as f64 / 1_048_576.0),
    );
    frame.render_widget(
        Paragraph::new(format!(
            "CPU: {cpu} (100% = one core)  |  RAM: {memory}  |  Players: {}",
            view.players.len()
        ))
        .block(Block::bordered().title(" Pumpkin | Server resources ")),
        stats,
    );
    let height = usize::from(console.height.saturating_sub(2));
    let logs: Vec<_> = {
        let logs = LOGS
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let end = logs
            .len()
            .saturating_sub(view.log_scroll.min(logs.len().saturating_sub(height)));
        logs.iter()
            .skip(end.saturating_sub(height))
            .take(height)
            .map(|line| {
                let color = if line.contains("ERROR") {
                    Color::Red
                } else if line.contains("WARN") {
                    Color::Yellow
                } else {
                    Color::White
                };
                Line::styled(line.clone(), Style::default().fg(color))
            })
            .collect()
    };
    frame.render_widget(
        Paragraph::new(logs).block(Block::bordered().title(" Console ")),
        console,
    );
    let names: Vec<_> = view
        .players
        .iter()
        .skip(view.player_scroll)
        .map(|(name, operator)| {
            ListItem::new(if *operator {
                format!("{name} [OP]")
            } else {
                name.clone()
            })
            .style(Style::default().fg(if *operator {
                Color::Yellow
            } else {
                Color::White
            }))
        })
        .collect();
    frame.render_widget(
        List::new(names).block(
            Block::bordered().title(format!(" Players ({}) | OP: yellow ", view.players.len())),
        ),
        players,
    );
    let width = usize::from(input.width.saturating_sub(2));
    let start = view.cursor.saturating_sub(width.saturating_sub(1));
    let text: String = view.input.iter().skip(start).collect();
    frame.render_widget(
        Paragraph::new(text).block(Block::bordered().title(if console_enabled {
            " Command "
        } else {
            " Console input disabled "
        })),
        input,
    );
    frame.render_widget(
        Paragraph::new(
            "Enter: run | Tab: complete | Up/Down: history | PgUp/PgDn: logs | Alt+Up/Down: players | Ctrl+C: stop",
        ),
        help,
    );
    if console_enabled && width > 0 && input.height >= 3 {
        let prefix: String = view.input[start..view.cursor].iter().collect();
        let cursor_width = Line::raw(prefix).width().min(width.saturating_sub(1));
        frame.set_cursor_position((input.x + 1 + cursor_width as u16, input.y + 1));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;

    #[test]
    fn history_restores_the_pending_command() {
        let mut view = View {
            history: VecDeque::from(["list".to_string(), "help".to_string()]),
            input: "say draft".chars().collect(),
            cursor: 9,
            ..View::default()
        };
        view.history(false);
        assert_eq!(view.input.iter().collect::<String>(), "say draft");
        view.history(true);
        assert_eq!(view.input.iter().collect::<String>(), "help");
        view.history(true);
        assert_eq!(view.input.iter().collect::<String>(), "list");
        view.history(false);
        view.history(false);
        assert_eq!(view.input.iter().collect::<String>(), "say draft");
        assert_eq!(view.cursor, 9);
    }

    #[test]
    fn dashboard_marks_operators_and_survives_small_terminals() {
        let view = View {
            players: vec![("Admin".to_string(), true), ("Guest".to_string(), false)],
            input: "say hello".chars().collect(),
            cursor: 9,
            ..View::default()
        };
        let mut terminal = Terminal::new(TestBackend::new(120, 24)).unwrap();
        terminal.draw(|frame| draw(frame, &view, true)).unwrap();
        let buffer = terminal.backend().buffer();
        let text: String = buffer
            .content
            .iter()
            .map(ratatui::buffer::Cell::symbol)
            .collect();
        assert!(text.contains("Admin [OP]"));
        assert!(text.contains("Guest"));
        assert!(text.contains("Players: 2"));
        assert_eq!(buffer[(91, 4)].fg, Color::Yellow);
        assert_eq!(buffer[(91, 5)].fg, Color::White);

        for (width, height) in [(1, 1), (8, 4), (30, 8)] {
            let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
            terminal.draw(|frame| draw(frame, &view, true)).unwrap();
        }
    }
}
