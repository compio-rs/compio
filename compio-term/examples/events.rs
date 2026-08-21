use std::io;

use compio_io::AsyncWrite;
use compio_runtime::Runtime;
use compio_term::{
    CommandQueue, Queueable, RawMode,
    event::{Event, EventStream, KeyCode, KeyEventKind, KeyModifiers},
    style::Print,
};
use futures_util::StreamExt;

#[compio_macros::main]
async fn main() -> io::Result<()> {
    let mut stdout = compio_fs::stdout();
    let mut output = stdout.queue(Print(format!(
        "Compio driver: {:?}\r\n",
        Runtime::current().driver_type()
    )))?;
    output.flush().await?;

    let raw_mode = RawMode::enable()?;
    let mut events = EventStream::new()?;
    terminal_line(
        &mut output,
        "reading terminal events; press q or Ctrl-C to stop",
    )
    .await?;

    while let Some(event) = events.next().await {
        let event = event?;
        terminal_line(&mut output, &format!("{event:?}")).await?;
        if should_quit(&event) {
            break;
        }
    }

    drop(events);
    drop(raw_mode);
    terminal_line(&mut output, "terminal restored").await?;
    Ok(())
}

fn should_quit(event: &Event) -> bool {
    let Event::Key(key) = event else {
        return false;
    };
    if key.kind != KeyEventKind::Press {
        return false;
    }
    key.code == KeyCode::Char('q')
        || key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL)
}

async fn terminal_line<W: AsyncWrite>(output: &mut CommandQueue<W>, line: &str) -> io::Result<()> {
    output.queue_many(&[Print(line), Print("\r\n")])?;
    output.flush().await
}
