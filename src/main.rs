use std::io;

use crossterm::{
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};

use graphql_client::GraphQLQuery;
use ratatui::{backend::CrosstermBackend, Terminal};
use tracing_subscriber::{fmt, prelude::*, EnvFilter};

use anyhow::Result;

mod get_itinerary;
mod get_location;

#[derive(GraphQLQuery)]
#[graphql(
    schema_path = "src/schema.graphql",
    query_path = "src/queries/routes.graphql",
    response_derives = "Debug"
)]
pub struct RoutesQuery;

#[tokio::main]
async fn main() -> Result<()> {
    let file_appender = tracing_appender::rolling::never(".", "client.log");
    let (non_blocking, _guard) = tracing_appender::non_blocking(file_appender);
    tracing_subscriber::registry()
        .with(fmt::layer().with_writer(non_blocking))
        .with(EnvFilter::from_default_env())
        .init();

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let from = get_location::get_location(&mut terminal).await?;
    let to = get_location::get_location(&mut terminal).await?;

    get_itinerary::get_itinerary(&mut terminal, from, to).await?;

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;

    Ok(())
}
