use std::{sync::Arc, time::Duration};

use anyhow::anyhow;
use anyhow::Result;
use crossterm::event::{self, Event, KeyCode};
use ratatui::{
    backend::Backend,
    layout::{Constraint, Direction, Layout},
    style::{Color, Style},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph},
    Terminal,
};
use reqwest::Client;
use serde::Deserialize;
use tokio::sync::{Notify, RwLock};
use unicode_width::UnicodeWidthStr;

#[derive(Deserialize, Debug, Clone)]
struct LocationResponse {
    features: Vec<Feature>,
}

#[derive(Deserialize, Debug, Clone)]
pub struct Feature {
    pub geometry: Geometry,
    pub properties: Properties,
}

#[derive(Deserialize, Debug, Clone)]
pub struct Geometry {
    pub coordinates: Vec<f64>,
}

#[derive(Deserialize, Debug, Clone)]
pub struct Properties {
    pub label: String,
}

async fn get_locations(client: &Client, query: &str) -> Result<LocationResponse> {
    let request = client
        .get("http://api.digitransit.fi/geocoding/v1/autocomplete")
        .header("digitransit-subscription-key", include_str!("../.apikey"))
        .query(&[("text", query)]);
    Ok(request.send().await?.json().await?)
}

pub async fn get_location<B: Backend>(terminal: &mut Terminal<B>) -> Result<Feature> {
    let input = Arc::new(RwLock::new(String::new()));
    let locations = Arc::new(RwLock::new(LocationResponse { features: vec![] }));

    let input_notify = Arc::new(Notify::new());

    let locations_task = {
        let input = input.clone();
        let locations = locations.clone();
        let input_notify = input_notify.clone();
        tokio::spawn(async move {
            let client = reqwest::Client::new();
            loop {
                input_notify.notified().await;
                let input = input.read().await.clone();
                let result = get_locations(&client, &input).await;
                if let Ok(result) = result {
                    tracing::info!("{:?}", result);
                    let mut locations = locations.write().await;
                    *locations = result;
                }
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
        })
    };

    let mut locations_state = ListState::default();

    loop {
        {
            let input = input.read().await.clone();
            let locations = locations.read().await.clone();
            terminal.draw(|frame| {
                let chunks = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([Constraint::Length(3), Constraint::Min(0)])
                    .margin(1)
                    .split(frame.size());

                let input_block = Paragraph::new(input.clone())
                    .block(Block::default().title("Location").borders(Borders::ALL));
                frame.set_cursor(chunks[0].x + input.width() as u16 + 1, chunks[0].y + 1);
                frame.render_widget(input_block, chunks[0]);

                let items: Vec<ListItem> = locations
                    .features
                    .iter()
                    .map(|feature| ListItem::new(feature.properties.label.clone()))
                    .collect();
                let results_block = List::new(items)
                    .highlight_style(Style::default().fg(Color::Black).bg(Color::White))
                    .block(Block::default().title("Locations").borders(Borders::ALL));
                frame.render_stateful_widget(results_block, chunks[1], &mut locations_state);
            })?;
        }

        if event::poll(Duration::from_millis(16))? {
            if let Event::Key(key) = event::read()? {
                match key.code {
                    KeyCode::Enter => break,
                    KeyCode::Char(c) => {
                        let mut input = input.write().await;
                        input.push(c);
                        input_notify.notify_one();
                    }
                    KeyCode::Backspace => {
                        let mut input = input.write().await;
                        input.pop();
                        input_notify.notify_one();
                    }
                    KeyCode::Up => {
                        let locations = locations.read().await;
                        if !locations.features.is_empty() {
                            let i = match locations_state.selected() {
                                Some(i) => {
                                    if i == 0 {
                                        locations.features.len() - 1
                                    } else {
                                        i - 1
                                    }
                                }
                                None => 0,
                            };
                            locations_state.select(Some(i));
                        }
                    }
                    KeyCode::Down => {
                        let locations = locations.read().await;
                        if !locations.features.is_empty() {
                            let i = match locations_state.selected() {
                                Some(i) => {
                                    if i >= locations.features.len() - 1 {
                                        0
                                    } else {
                                        i + 1
                                    }
                                }
                                None => 0,
                            };
                            locations_state.select(Some(i));
                        }
                    }
                    _ => (),
                }
            }
        }
    }

    locations_task.abort();

    if let Some(selected) = locations_state.selected() {
        let location = &locations.read().await.features[selected];
        Ok(location.clone())
    } else {
        Err(anyhow!("Missing location selection"))
    }
}
