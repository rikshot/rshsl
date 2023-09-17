use std::{
    sync::{atomic::AtomicBool, Arc},
    time::Duration,
};

use anyhow::Result;
use chrono::{Local, TimeZone};
use crossterm::event::{self, Event, KeyCode};
use graphql_client::{GraphQLQuery, Response};
use ratatui::{
    backend::Backend,
    layout::{Alignment, Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Terminal,
};
use reqwest::Client;
use std::sync::atomic::Ordering::Relaxed;
use tokio::sync::RwLock;
use tracing::info;

use crate::get_location::Feature;

use self::plan_query::{
    InputCoordinates, Mode, PlanQueryPlanItineraries, PlanQueryPlanItinerariesLegs,
};

type Long = u64;

#[derive(GraphQLQuery)]
#[graphql(
    schema_path = "src/schema.graphql",
    query_path = "src/queries/plan.graphql",
    response_derives = "Debug,PartialEq"
)]
pub struct PlanQuery;

fn format_duration(duration: &Duration) -> String {
    let seconds = duration.as_secs();
    let hours = seconds / 3600;
    let minutes = seconds % 3600 / 60;
    let seconds = seconds % 3600 % 60;
    [
        if hours > 0 { format!("{}h ", hours) } else { String::new() },
        if minutes > 0 { format!("{}m ", minutes) } else { String::new() },
        if seconds > 0 { format!("{}s ", seconds) } else { String::new() },
    ]
    .join("")
    .trim()
    .to_string()
}

fn format_title(itinerary: &PlanQueryPlanItineraries) -> String {
    format!(
        "[ {} - {} | {} ]",
        Local
            .timestamp_opt(itinerary.start_time.unwrap() as i64 / 1000, 0)
            .single()
            .unwrap()
            .format("%H:%M"),
        Local
            .timestamp_opt(itinerary.end_time.unwrap() as i64 / 1000, 0)
            .single()
            .unwrap()
            .format("%H:%M"),
        format_duration(&Duration::from_secs(itinerary.duration.unwrap()))
    )
}

pub async fn get_itinerary<B: Backend>(
    terminal: &mut Terminal<B>,
    from: Feature,
    to: Feature,
) -> Result<()> {
    let form_coordinates = InputCoordinates {
        lat: from.geometry.coordinates[1],
        lon: from.geometry.coordinates[0],
        address: Some(from.properties.label.clone()),
        location_slack: None,
    };
    let to_coordinates = InputCoordinates {
        lat: to.geometry.coordinates[1],
        lon: to.geometry.coordinates[0],
        address: Some(to.properties.label.clone()),
        location_slack: None,
    };

    let itineraries = Arc::new(RwLock::new(vec![]));

    let updating = Arc::new(AtomicBool::new(false));
    let itineraries_task: tokio::task::JoinHandle<Result<()>> = {
        let updating = updating.clone();
        let itineraries = itineraries.clone();
        tokio::spawn(async move {
            let client = Client::new();
            let body = PlanQuery::build_query(plan_query::Variables {
                from: form_coordinates,
                to: to_coordinates,
            });

            loop {
                {
                    info!("Updating itineraries...");
                    updating.store(true, Relaxed);
                    let response: Response<plan_query::ResponseData> = client
                        .post("https://api.digitransit.fi/routing/v1/routers/hsl/index/graphql")
                        .header("digitransit-subscription-key", include_str!("../.apikey"))
                        .json(&body)
                        .send()
                        .await?
                        .json()
                        .await?;

                    *itineraries.write().await = response.data.unwrap().plan.unwrap().itineraries;
                    updating.store(false, Relaxed);
                }

                tokio::time::sleep(Duration::from_secs(60)).await;
            }
        })
    };

    loop {
        {
            let itineraries = itineraries.read().await;
            terminal.draw(|frame| {
                let chunks = Layout::default()
                    .direction(Direction::Vertical)
                    .margin(1)
                    .constraints(
                        [
                            vec![Constraint::Length(2)],
                            vec![Constraint::Length(5); itineraries.len()],
                            vec![Constraint::Max(0)],
                        ]
                        .concat(),
                    )
                    .split(frame.size());

                let title_chunks = Layout::default()
                    .direction(Direction::Horizontal)
                    .constraints([Constraint::Ratio(1, 2), Constraint::Ratio(1, 2)])
                    .split(chunks[0]);

                let title_block =
                    Paragraph::new(format!("{} -> {}", from.properties.label, to.properties.label));
                frame.render_widget(title_block, title_chunks[0]);

                let status_block =
                    Paragraph::new(if updating.load(Relaxed) { "Updating..." } else { "Idle" })
                        .alignment(Alignment::Right);
                frame.render_widget(status_block, title_chunks[1]);

                for (index, itinerary) in itineraries.iter().enumerate() {
                    if let Some(itinerary) = itinerary {
                        let itinerary_block = Block::default()
                            .title(Span::styled(
                                format_title(itinerary),
                                Style::default().add_modifier(Modifier::BOLD),
                            ))
                            .borders(Borders::ALL);

                        let legs: Vec<&Option<PlanQueryPlanItinerariesLegs>> = itinerary
                            .legs
                            .iter()
                            .filter(|leg| {
                                if let Some(leg) = leg {
                                    leg.duration.unwrap() > 60.0
                                } else {
                                    false
                                }
                            })
                            .collect();

                        let constraints = legs
                            .iter()
                            .map(|leg| {
                                Constraint::Ratio(
                                    (leg.as_ref().unwrap().duration.unwrap()
                                        / itinerary.duration.unwrap() as f64
                                        * 100.0) as u32,
                                    100,
                                )
                            })
                            .collect::<Vec<Constraint>>();

                        let leg_chunks = Layout::default()
                            .direction(Direction::Horizontal)
                            .constraints(constraints)
                            .split(itinerary_block.inner(chunks[index + 1]));

                        for (index, leg) in legs.iter().enumerate() {
                            let mode = leg.as_ref().unwrap().mode.as_ref().unwrap();
                            let from_stop_name: &str = if *mode != Mode::WALK {
                                leg.as_ref().unwrap().from.stop.as_ref().unwrap().name.as_ref()
                            } else {
                                ""
                            };
                            let to_stop_name: &str = if *mode != Mode::WALK {
                                leg.as_ref().unwrap().to.stop.as_ref().unwrap().name.as_ref()
                            } else {
                                ""
                            };
                            frame.render_widget(
                                Paragraph::new(vec![
                                    Line::from(Span::styled(
                                        from_stop_name,
                                        Style::default().add_modifier(Modifier::REVERSED),
                                    )),
                                    Line::from(Span::raw(if *mode == Mode::WALK {
                                        format!(
                                            "\u{1F6B6} {}",
                                            format_duration(&Duration::from_secs_f64(
                                                leg.as_ref().unwrap().duration.unwrap()
                                            ))
                                        )
                                    } else {
                                        format!(
                                            "{} ({}) {}",
                                            match mode {
                                                Mode::BUS => "\u{1F68C}",
                                                Mode::RAIL => "\u{1F686}",
                                                Mode::SUBWAY => "\u{1F687}",
                                                _ => "none",
                                            },
                                            leg.as_ref()
                                                .unwrap()
                                                .route
                                                .as_ref()
                                                .unwrap()
                                                .short_name
                                                .as_ref()
                                                .unwrap(),
                                            format_duration(&Duration::from_secs_f64(
                                                leg.as_ref().unwrap().duration.unwrap()
                                            ))
                                        )
                                    })),
                                    Line::from(Span::styled(
                                        to_stop_name,
                                        Style::default().add_modifier(Modifier::REVERSED),
                                    )),
                                ])
                                .alignment(Alignment::Center)
                                .style(Style::default().bg(
                                    match mode {
                                        Mode::WALK => Color::Black,
                                        Mode::BUS => Color::Blue,
                                        Mode::RAIL => Color::Magenta,
                                        _ => Color::Black,
                                    },
                                )),
                                leg_chunks[index],
                            );
                        }

                        frame.render_widget(itinerary_block, chunks[index + 1]);
                    }
                }
            })?;
        }

        if event::poll(Duration::from_millis(16))? {
            if let Event::Key(key) = event::read()? {
                match key.code {
                    KeyCode::Char('q') => break,
                    KeyCode::Esc => break,
                    _ => (),
                }
            }
        }
    }

    itineraries_task.abort();

    Ok(())
}
