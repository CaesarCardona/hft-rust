use std::io::{self, stdout};
use std::sync::{Arc, RwLock};
use std::thread;
use std::time::{Duration, Instant};

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
    widgets::{Block, Borders, Paragraph, Chart, Dataset, Axis},
    symbols,
    Terminal,
};

const HISTORY_LEN: usize = 100;

#[derive(Clone)]
struct MarketData {
    count: usize,
    price: f64,
    last_update: Instant,
    history: Vec<f64>,
}

#[derive(Clone)]
struct UiData {
    count: usize,
    value: f64,
    last_update: Instant,
    history: Vec<f64>,
}

fn rolling_avg(values: &[f64]) -> f64 {
    if values.is_empty() { 0.0 } else { values.iter().sum::<f64>() / values.len() as f64 }
}

fn main() -> io::Result<()> {
    let n_stocks = 3;

    // --- Colors for each stock ---
    let colors = [Color::Red, Color::Green, Color::Yellow];

    // --- HFT market data ---
    let market_data = Arc::new(RwLock::new(
        (0..n_stocks)
            .map(|i| MarketData {
                count: i,
                price: 100.0,
                last_update: Instant::now(),
                history: vec![100.0; HISTORY_LEN],
            })
            .collect::<Vec<_>>(),
    ));

    // --- Frontend data ---
    let ui_data = Arc::new(RwLock::new(
        (0..n_stocks)
            .map(|i| UiData {
                count: i,
                value: 100.0,
                last_update: Instant::now(),
                history: vec![100.0; HISTORY_LEN],
            })
            .collect::<Vec<_>>()
    ));

    // --- HFT updater (~100Hz) ---
    {
        let md_clone = Arc::clone(&market_data);
        thread::spawn(move || {
            let mut rng = rand::thread_rng();
            loop {
                {
                    let mut vec = md_clone.write().unwrap();
                    for md in vec.iter_mut() {
                        let delta = rng.gen_range(-1.0..1.0);
                        md.price = (md.price + delta).max(0.0);
                        md.last_update = Instant::now();
                        md.history.push(md.price);
                        if md.history.len() > HISTORY_LEN { md.history.remove(0); }
                    }
                }
                thread::sleep(Duration::from_millis(10));
            }
        });
    }

    // --- Frontend updater (~2Hz) ---
    {
        let ui_clone = Arc::clone(&ui_data);
        thread::spawn(move || {
            let mut rng = rand::thread_rng();
            loop {
                {
                    let mut vec = ui_clone.write().unwrap();
                    for ui in vec.iter_mut() {
                        let delta = rng.gen_range(-2.0..2.0);
                        ui.value += delta;
                        ui.last_update = Instant::now();
                        ui.history.push(ui.value);
                        if ui.history.len() > HISTORY_LEN { ui.history.remove(0); }
                    }
                }
                thread::sleep(Duration::from_millis(500));
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
                if key.code == KeyCode::Char('q') { break; }
            }
        }

        let md_vec = market_data.read().unwrap().clone();
        let ui_vec = ui_data.read().unwrap().clone();

        terminal.draw(|f| {
            let main_chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
                .split(f.area());

            // --- HFT Panel ---
            let hft_chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Length(n_stocks as u16 * 2), Constraint::Min(5)])
                .split(main_chunks[0]);

            // HFT numeric display
            let mut hft_lines = vec![];
            for (i, md) in md_vec.iter().enumerate() {
                let avg = rolling_avg(&md.history);
                let ui_avg = rolling_avg(&ui_vec[i].history);
                let diff = avg - ui_avg;
                hft_lines.push(ratatui::text::Line::from(vec![
                    ratatui::text::Span::styled(format!("0x{:X}: ", md.count), Style::default().fg(colors[i])),
                    ratatui::text::Span::raw(format!(
                        "Price: {:.2}, Latency: {}µs, Avg(100): {:.2} | Frontend Avg: {:.2}, Δ: {:.2}",
                        md.price,
                        md.last_update.elapsed().as_micros(),
                        avg,
                        ui_avg,
                        diff
                    )),
                ]));
            }
            f.render_widget(
                Paragraph::new(hft_lines).block(Block::default().title("📈 HFT Data").borders(Borders::ALL)),
                hft_chunks[0],
            );

            // HFT chart
            let hft_points: Vec<Vec<(f64,f64)>> = md_vec.iter()
                .map(|md| md.history.iter().enumerate().map(|(i, y)| (i as f64, *y)).collect())
                .collect();
            let datasets: Vec<Dataset> = md_vec.iter().enumerate().map(|(i, _)| {
                Dataset::default()
                    .name(format!("0x{:X}", i))
                    .marker(symbols::Marker::Dot)
                    .style(Style::default().fg(colors[i]))
                    .data(&hft_points[i])
            }).collect();
            let min_y = md_vec.iter().flat_map(|x| x.history.iter()).cloned().fold(f64::INFINITY,f64::min)-1.0;
            let max_y = md_vec.iter().flat_map(|x| x.history.iter()).cloned().fold(f64::NEG_INFINITY,f64::max)+1.0;
            let hft_chart = Chart::new(datasets)
                .block(Block::default().borders(Borders::ALL).title("HFT History"))
                .x_axis(Axis::default().bounds([0.0,HISTORY_LEN as f64]))
                .y_axis(Axis::default().bounds([min_y,max_y]));
            f.render_widget(hft_chart,hft_chunks[1]);

            // --- Frontend Panel ---
            let ui_chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Length(n_stocks as u16 * 2), Constraint::Min(5)])
                .split(main_chunks[1]);

            // UI numeric display
            let mut ui_lines = vec![];
            for (i, ui) in ui_vec.iter().enumerate() {
                let avg = rolling_avg(&ui.history);
                ui_lines.push(ratatui::text::Line::from(vec![
                    ratatui::text::Span::styled(format!("0x{:X}: ", ui.count), Style::default().fg(colors[i])),
                    ratatui::text::Span::raw(format!("Value: {:.2}, Latency: {}µs, Avg(100): {:.2}", ui.value, ui.last_update.elapsed().as_micros(), avg)),
                ]));
            }
            f.render_widget(
                Paragraph::new(ui_lines).block(Block::default().title("🧩 Frontend Data").borders(Borders::ALL)),
                ui_chunks[0],
            );

            // UI chart
            let ui_points: Vec<Vec<(f64,f64)>> = ui_vec.iter()
                .map(|ui| ui.history.iter().enumerate().map(|(i, y)| (i as f64, *y)).collect())
                .collect();
            let ui_datasets: Vec<Dataset> = ui_vec.iter().enumerate().map(|(i, _)| {
                Dataset::default()
                    .name(format!("0x{:X}", i))
                    .marker(symbols::Marker::Braille)
                    .style(Style::default().fg(colors[i]))
                    .data(&ui_points[i])
            }).collect();
            let min_y_ui = ui_vec.iter().flat_map(|x| x.history.iter()).cloned().fold(f64::INFINITY,f64::min)-1.0;
            let max_y_ui = ui_vec.iter().flat_map(|x| x.history.iter()).cloned().fold(f64::NEG_INFINITY,f64::max)+1.0;
            let ui_chart = Chart::new(ui_datasets)
                .block(Block::default().borders(Borders::ALL).title("Frontend History"))
                .x_axis(Axis::default().bounds([0.0,HISTORY_LEN as f64]))
                .y_axis(Axis::default().bounds([min_y_ui,max_y_ui]));
            f.render_widget(ui_chart,ui_chunks[1]);
        })?;

        thread::sleep(Duration::from_millis(50));
    }

    disable_raw_mode()?;
    terminal.backend_mut().execute(LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    Ok(())
}

