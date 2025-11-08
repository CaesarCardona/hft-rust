use std::fs::OpenOptions;
use std::io::{self, stdout, Write};
use std::sync::{Arc, RwLock};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crossterm::{
    event::{self, Event, KeyCode},
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
    ExecutableCommand,
};
use rand::Rng;
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout},
    style::{Color, Style},
    symbols,
    widgets::{Block, Borders, Paragraph, Chart, Dataset, Axis},
    Terminal,
};

const HISTORY_LEN: usize = 50;
const MOVING_AVG_LEN: usize = 5;

#[derive(Clone)]
struct MarketData {
    count: usize,
    price: Arc<RwLock<f64>>, // backend fixed pointer
    last_update: Instant,
    history: Vec<f64>,
}

#[derive(Clone)]
struct UiData {
    count: usize,
    value: f64, // no pointer needed for file saving
    last_update: Instant,
    history: Vec<f64>,
}

fn main() -> io::Result<()> {
    let n_stocks = 3;
    let colors = [Color::Red, Color::Green, Color::Yellow];

    // --- Backend ---
    let market_data = Arc::new(RwLock::new(
        (0..n_stocks)
            .map(|i| {
                let init = 100.0;
                MarketData {
                    count: i,
                    price: Arc::new(RwLock::new(init)),
                    last_update: Instant::now(),
                    history: vec![init; HISTORY_LEN],
                }
            })
            .collect::<Vec<_>>(),
    ));

    // --- Frontend ---
    let ui_data = Arc::new(RwLock::new(
        (0..n_stocks)
            .map(|i| UiData {
                count: i,
                value: 100.0,
                last_update: Instant::now(),
                history: vec![],
            })
            .collect::<Vec<_>>(),
    ));

    // --- Backend updater ---
    {
        let md_clone = Arc::clone(&market_data);
        thread::spawn(move || {
            let mut rng = rand::thread_rng();
            loop {
                {
                    let mut vec = md_clone.write().unwrap();
                    for md in vec.iter_mut() {
                        let delta = rng.gen_range(-2.0..2.0);
                        let mut p = md.price.write().unwrap();
                        *p += delta; // pointer fixed
                        md.last_update = Instant::now();
                        md.history.push(*p);
                        if md.history.len() > HISTORY_LEN {
                            md.history.remove(0);
                        }
                    }
                }
                thread::sleep(Duration::from_millis(100));
            }
        });
    }

    // --- Frontend updater (moving average + save to file) ---
    {
        let md_clone = Arc::clone(&market_data);
        let ui_clone = Arc::clone(&ui_data);

        thread::spawn(move || {
            loop {
                {
                    let md_vec = md_clone.read().unwrap();
                    let mut ui_vec = ui_clone.write().unwrap();

                    for (i, ui) in ui_vec.iter_mut().enumerate() {
                        let len = md_vec[i].history.len();
                        let start = len.saturating_sub(MOVING_AVG_LEN);
                        let slice = &md_vec[i].history[start..];
                        let avg = slice.iter().sum::<f64>() / slice.len() as f64;

                        ui.value = avg;
                        ui.last_update = Instant::now();
                        ui.history.push(avg);
                        if ui.history.len() > HISTORY_LEN {
                            ui.history.remove(0);
                        }

                        // --- Save moving average to file ---
                        let now = SystemTime::now()
                            .duration_since(UNIX_EPOCH)
                            .expect("Time went backwards");
                        let timestamp_ns = now.as_secs() * 1_000_000_000 + now.subsec_nanos() as u64;
                        let line = format!("{},{:.10}\n", timestamp_ns, avg);

                        let mut file = OpenOptions::new()
                            .append(true)
                            .create(true)
                            .open("moving_avg.txt")
                            .unwrap();
                        file.write_all(line.as_bytes()).unwrap();
                    }
                }
                thread::sleep(Duration::from_millis(300));
            }
        });
    }

    // --- Terminal setup ---
    enable_raw_mode()?;
    let mut stdout = stdout();
    stdout.execute(EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    loop {
        if event::poll(Duration::from_millis(10))? {
            if let Event::Key(key) = event::read()? {
                if key.code == KeyCode::Char('q') {
                    break;
                }
            }
        }

        let md_vec = market_data.read().unwrap().clone();
        let ui_vec = ui_data.read().unwrap().clone();

        terminal.draw(|f| {
            let main_chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Length(8), Constraint::Min(10)])
                .split(f.area());

            // --- Pointers display (optional) ---
            let mut lines = vec![];
            for md in md_vec.iter() {
                let val = *md.price.read().unwrap();
                lines.push(ratatui::text::Line::from(format!(
                    "Backend Stock {} -> ptr: {:p}, value: {:.2}",
                    md.count,
                    Arc::as_ptr(&md.price),
                    val
                )));
            }
            for ui in ui_vec.iter() {
                lines.push(ratatui::text::Line::from(format!(
                    "Frontend Stock {} -> moving avg: {:.2}",
                    ui.count,
                    ui.value
                )));
            }
            f.render_widget(
                Paragraph::new(lines)
                    .block(Block::default().borders(Borders::ALL).title("Pointers / Values")),
                main_chunks[0],
            );

            // --- Chart ---
            let chart_chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Percentage(100)])
                .split(main_chunks[1]);

            let md_points: Vec<Vec<(f64, f64)>> = md_vec
                .iter()
                .map(|md| md.history.iter().enumerate().map(|(i, y)| (i as f64, *y)).collect())
                .collect();

            let ui_points: Vec<Vec<(f64, f64)>> = ui_vec
                .iter()
                .map(|ui| ui.history.iter().enumerate().map(|(i, y)| (i as f64, *y)).collect())
                .collect();

            let datasets: Vec<Dataset> = md_points
                .iter()
                .enumerate()
                .map(|(i, pts)| {
                    Dataset::default()
                        .name(format!("Backend {}", i))
                        .marker(symbols::Marker::Dot)
                        .style(Style::default().fg(colors[i]))
                        .data(pts)
                })
                .chain(ui_points.iter().enumerate().map(|(i, pts)| {
                    Dataset::default()
                        .name(format!("Frontend {}", i))
                        .marker(symbols::Marker::Braille)
                        .style(Style::default().fg(colors[i]))
                        .data(pts)
                }))
                .collect();

            let min_y = md_vec.iter()
                .flat_map(|x| x.history.iter())
                .chain(ui_vec.iter().flat_map(|x| x.history.iter()))
                .cloned()
                .fold(f64::INFINITY, f64::min) - 1.0;

            let max_y = md_vec.iter()
                .flat_map(|x| x.history.iter())
                .chain(ui_vec.iter().flat_map(|x| x.history.iter()))
                .cloned()
                .fold(f64::NEG_INFINITY, f64::max) + 1.0;

            let chart = Chart::new(datasets)
                .block(Block::default().borders(Borders::ALL).title("Stock Values"))
                .x_axis(Axis::default().bounds([0.0, HISTORY_LEN as f64]))
                .y_axis(Axis::default().bounds([min_y, max_y]));

            f.render_widget(chart, chart_chunks[0]);
        })?;

        thread::sleep(Duration::from_millis(50));
    }

    disable_raw_mode()?;
    terminal.backend_mut().execute(LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    Ok(())
}

