use std::io::{BufRead, BufReader, Write};
use std::path::Path;

use crate::event::Event;
use crate::Result;

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
        let reader = BufReader::new(file);
        let mut events = Vec::new();
        for line in reader.lines() {
            let line = line?;
            let event: Event = serde_json::from_str(&line)?;
            events.push(event);
        }
        Ok(Self { events })
    }
}
