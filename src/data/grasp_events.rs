use std::collections::{HashMap, HashSet};

use thiserror::Error;

/// The three timed events of a grasp, in the order the CSV lists them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GraspEventKind {
    ReachOnset,
    FirstContact,
    GripEstablished,
}

impl GraspEventKind {
    pub const ALL: [GraspEventKind; 3] = [Self::ReachOnset, Self::FirstContact, Self::GripEstablished];

    pub fn code(self) -> &'static str {
        match self {
            Self::ReachOnset => "E2",
            Self::FirstContact => "E3",
            Self::GripEstablished => "E4",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::ReachOnset => "reach onset",
            Self::FirstContact => "first contact",
            Self::GripEstablished => "grip established",
        }
    }
}

/// How sure the coder was of an event time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Certainty {
    Clear,
    Probable,
    Inferred,
    NotObservable,
    Other(String),
}

impl Certainty {
    fn parse(cell: &str) -> Self {
        match cell {
            "clear" => Self::Clear,
            "probable" => Self::Probable,
            "inferred" => Self::Inferred,
            "not_observable" => Self::NotObservable,
            other => Self::Other(other.to_string()),
        }
    }

    pub fn label(&self) -> &str {
        match self {
            Self::Clear => "clear",
            Self::Probable => "probable",
            Self::Inferred => "inferred",
            Self::NotObservable => "not observable",
            Self::Other(text) => text,
        }
    }
}

/// One event's time (seconds from the start of the trial video) and certainty; a blank time means no marker.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct GraspEvent {
    pub time: Option<f64>,
    pub certainty: Option<Certainty>,
}

/// How one hand grips the object. Every cell is `None` when the CSV left it blank.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct HandGrip {
    pub used: Option<bool>,
    pub grip_type: Option<String>,
    pub thumb: Option<String>,
    pub index: Option<String>,
    pub middle: Option<String>,
    pub ring: Option<String>,
    pub little: Option<String>,
    pub palm_contact: Option<String>,
    pub thumb_opposition: Option<bool>,
}

impl HandGrip {
    /// The digit contacts as `(name, value)` in thumb-to-little order.
    pub fn digits(&self) -> [(&'static str, Option<&str>); 5] {
        [
            ("Thumb", self.thumb.as_deref()),
            ("Index", self.index.as_deref()),
            ("Middle", self.middle.as_deref()),
            ("Ring", self.ring.as_deref()),
            ("Little", self.little.as_deref()),
        ]
    }

    pub fn has_detail(&self) -> bool {
        self.grip_type.is_some()
            || self.palm_contact.is_some()
            || self.thumb_opposition.is_some()
            || self.digits().iter().any(|(_, value)| value.is_some())
    }
}

/// An event placed on a frame of the open trial, ready to draw.
#[derive(Debug, Clone, PartialEq)]
pub struct GraspMark {
    pub kind: GraspEventKind,
    pub frame: usize,
    pub certainty: Option<Certainty>,
}

/// One trial's row of a grasp-events CSV.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct GraspRow {
    pub subject: String,
    pub trial: String,
    pub object_size: Option<String>,
    pub object_shape: Option<String>,
    pub object_color: Option<String>,
    pub contact_before_grip: Option<bool>,
    pub contact_status: Option<String>,
    pub left: HandGrip,
    pub right: HandGrip,
    pub events: [GraspEvent; 3],
}

impl GraspRow {
    pub fn event(&self, kind: GraspEventKind) -> &GraspEvent {
        &self.events[kind as usize]
    }

    /// The events that have a time and land inside a video of `frame_count` frames at `fps`. Times may be in any order.
    pub fn marks(&self, fps: f64, frame_count: usize) -> Vec<GraspMark> {
        if !(fps.is_finite() && fps > 0.0) {
            return Vec::new();
        }
        GraspEventKind::ALL
            .into_iter()
            .filter_map(|kind| {
                let event = self.event(kind);
                let frame = (event.time? * fps).round() as usize;
                (frame < frame_count).then(|| GraspMark {
                    kind,
                    frame,
                    certainty: event.certainty.clone(),
                })
            })
            .collect()
    }
}

#[derive(Debug, Error, PartialEq)]
pub enum GraspEventsError {
    #[error("{source_name}: the file is empty")]
    Empty { source_name: String },
    #[error("{source_name}: the header has no '{column}' column")]
    MissingColumn { source_name: String, column: &'static str },
    #[error("{source_name}: line {line}: subject and trial must not be blank")]
    BlankKey { source_name: String, line: usize },
    #[error("{source_name}: line {line}: {subject} / {trial} appears more than once")]
    DuplicateTrial {
        source_name: String,
        line: usize,
        subject: String,
        trial: String,
    },
    #[error("{source_name}: line {line}: '{value}' in column '{column}' is not a time in seconds")]
    BadTime {
        source_name: String,
        line: usize,
        column: &'static str,
        value: String,
    },
    #[error("{source_name}: {message}")]
    Csv { source_name: String, message: String },
}

/// A parsed grasp-events CSV: one row per trial.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct GraspFile {
    rows: HashMap<(String, String), GraspRow>,
}

impl GraspFile {
    pub fn len(&self) -> usize {
        self.rows.len()
    }

    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    pub fn row(&self, subject: &str, trial: &str) -> Option<&GraspRow> {
        self.rows.get(&(subject.to_string(), trial.to_string()))
    }

    /// Parses CSV text; `source_name` names where it came from, for error messages.
    pub fn parse(text: &str, source_name: &str) -> Result<Self, GraspEventsError> {
        let csv_error = |err: csv::Error| GraspEventsError::Csv {
            source_name: source_name.to_string(),
            message: err.to_string(),
        };
        let mut reader = csv::ReaderBuilder::new().flexible(true).from_reader(text.as_bytes());
        let header = reader.headers().map_err(csv_error)?.clone();
        if header.is_empty() {
            return Err(GraspEventsError::Empty {
                source_name: source_name.to_string(),
            });
        }
        let mut columns: HashMap<String, usize> = HashMap::new();
        for (index, name) in header.iter().enumerate() {
            columns.entry(name.trim().to_string()).or_insert(index);
        }
        for column in ["subject", "trial"] {
            if !columns.contains_key(column) {
                return Err(GraspEventsError::MissingColumn {
                    source_name: source_name.to_string(),
                    column,
                });
            }
        }

        let mut rows = HashMap::new();
        for record in reader.records() {
            let cells = record.map_err(csv_error)?;
            let line = cells.position().map_or(0, |p| p.line() as usize);
            let cell = |name: &str| -> Option<&str> {
                let value = cells.get(*columns.get(name)?)?.trim();
                (!value.is_empty()).then_some(value)
            };
            let (Some(subject), Some(trial)) = (cell("subject"), cell("trial")) else {
                return Err(GraspEventsError::BlankKey {
                    source_name: source_name.to_string(),
                    line,
                });
            };
            let text = |name: &str| cell(name).map(str::to_string);
            let flag = |name: &str| match cell(name)? {
                "true" => Some(true),
                "false" => Some(false),
                _ => None,
            };
            let hand = |side: &str| HandGrip {
                used: flag(&format!("{side}_hand_used")),
                grip_type: text(&format!("{side}_grip_type")),
                thumb: text(&format!("{side}_thumb")),
                index: text(&format!("{side}_index")),
                middle: text(&format!("{side}_middle")),
                ring: text(&format!("{side}_ring")),
                little: text(&format!("{side}_little")),
                palm_contact: text(&format!("{side}_palm_contact")),
                thumb_opposition: flag(&format!("{side}_thumb_opposition")),
            };
            let mut events: [GraspEvent; 3] = Default::default();
            for (event, (column, certainty)) in
                events
                    .iter_mut()
                    .zip([("e2", "e2_certainty"), ("e3", "e3_certainty"), ("e4", "e4_certainty")])
            {
                event.time = match cell(column) {
                    None => None,
                    Some(value) => match value.parse::<f64>() {
                        Ok(seconds) if seconds.is_finite() && seconds >= 0.0 => Some(seconds),
                        _ => {
                            return Err(GraspEventsError::BadTime {
                                source_name: source_name.to_string(),
                                line,
                                column,
                                value: value.to_string(),
                            });
                        }
                    },
                };
                event.certainty = cell(certainty).map(Certainty::parse);
            }
            let row = GraspRow {
                subject: subject.to_string(),
                trial: trial.to_string(),
                object_size: text("object_size"),
                object_shape: text("object_shape"),
                object_color: text("object_color"),
                contact_before_grip: flag("contact_before_grip"),
                contact_status: text("contact_status"),
                left: hand("left"),
                right: hand("right"),
                events,
            };
            let key = (row.subject.clone(), row.trial.clone());
            if rows.contains_key(&key) {
                return Err(GraspEventsError::DuplicateTrial {
                    source_name: source_name.to_string(),
                    line,
                    subject: key.0,
                    trial: key.1,
                });
            }
            rows.insert(key, row);
        }
        Ok(Self { rows })
    }

    pub fn from_bytes(bytes: &[u8], source_name: &str) -> Result<Self, GraspEventsError> {
        Self::parse(&String::from_utf8_lossy(bytes), source_name)
    }
}

/// The rows from every loaded file, resolved by priority.
#[derive(Debug, Clone, Default)]
pub struct GraspEventSet {
    rows: HashMap<(String, String), GraspRow>,
}

impl GraspEventSet {
    /// Combines `files`, ordered from highest priority to lowest. A trial present in several files takes the whole
    /// row of the first one that has it, blank cells included.
    pub fn merge<'a>(files: impl IntoIterator<Item = &'a GraspFile>) -> Self {
        let mut rows: HashMap<(String, String), GraspRow> = HashMap::new();
        let mut seen: HashSet<(String, String)> = HashSet::new();
        for file in files {
            for (key, row) in &file.rows {
                if seen.insert(key.clone()) {
                    rows.insert(key.clone(), row.clone());
                }
            }
        }
        Self { rows }
    }

    pub fn len(&self) -> usize {
        self.rows.len()
    }

    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    pub fn row(&self, subject: &str, trial: &str) -> Option<&GraspRow> {
        self.rows.get(&(subject.to_string(), trial.to_string()))
    }
}
