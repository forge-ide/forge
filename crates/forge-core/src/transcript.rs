use std::io::{BufRead, BufReader, Read, Write};
use std::path::Path;

use crate::event::Event;
use crate::event_log::MAX_LINE_BYTES;
use crate::{ForgeError, Result};

#[derive(Debug, Default)]
pub struct Transcript {
    events: Vec<Event>,
}

impl Transcript {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn append(&mut self, event: Event) {
        self.events.push(event);
    }

    pub fn events(&self) -> &[Event] {
        &self.events
    }

    pub fn to_file(&self, path: &Path) -> Result<()> {
        let mut file = std::fs::File::create(path)?;
        for event in &self.events {
            let line = serde_json::to_string(event)?;
            writeln!(file, "{line}")?;
        }
        Ok(())
    }

    pub fn from_file(path: &Path) -> Result<Self> {
        let file = std::fs::File::open(path)?;
        let mut reader = BufReader::new(file);
        let mut events = Vec::new();
        let mut line_num: u64 = 0;
        let mut buf: Vec<u8> = Vec::new();
        loop {
            buf.clear();
            // Cap per-line reads at MAX_LINE_BYTES. `std::io::Take<R: BufRead>`
            // implements `BufRead`, so `read_until` reuses the standard code path
            // while hard-limiting how many bytes the reader will hand back.
            let mut handle = reader.by_ref().take((MAX_LINE_BYTES as u64) + 1);
            let n = handle.read_until(b'\n', &mut buf)?;
            if n == 0 {
                break; // EOF
            }
            line_num += 1;
            // If we read MAX+1 bytes and the last byte is not '\n', the cap was
            // hit mid-line. Anything that stopped at the cap *with* a trailing
            // newline is exactly MAX content bytes — allowed.
            let ended_with_newline = buf.last() == Some(&b'\n');
            if n > MAX_LINE_BYTES && !ended_with_newline {
                return Err(ForgeError::Other(anyhow::anyhow!(
                    "transcript line {line_num} exceeds {MAX_LINE_BYTES} bytes"
                )));
            }
            let content = if ended_with_newline {
                &buf[..buf.len() - 1]
            } else {
                &buf[..]
            };
            let line = std::str::from_utf8(content).map_err(|_| {
                ForgeError::Other(anyhow::anyhow!(
                    "transcript line {line_num} is not valid UTF-8"
                ))
            })?;
            let event: Event = serde_json::from_str(line)?;
            events.push(event);
        }
        Ok(Self { events })
    }
}
