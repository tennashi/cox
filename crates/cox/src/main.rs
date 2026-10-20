//! The `cox` command: `cox <agent command> [args...]` starts the agent and
//! talks to it.

use agent_client_protocol::AcpAgent;
use anyhow::Context;
use tokio::io::BufReader;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let agent = AcpAgent::from_args(std::env::args().skip(1))
        .context("usage: cox <agent command> [args...]")?;
    let input = BufReader::new(tokio::io::stdin());
    cox::run(agent, input, &mut std::io::stdout()).await?;
    Ok(())
}
