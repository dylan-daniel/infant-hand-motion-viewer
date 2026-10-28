use serde::Deserialize;

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
    pub fn parse(cell: &str) -> Self {
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

/// An event placed on a frame of the open trial, ready to draw.
#[derive(Debug, Clone, PartialEq)]
pub struct GraspMark {
    pub kind: GraspEventKind,
    pub frame: usize,
    pub certainty: Option<Certainty>,
}

/// The first event frame after `from` and up to `to` (inclusive), where playback stepping from `from` to `to` should stop.
pub fn next_event_frame(marks: &[GraspMark], from: usize, to: usize) -> Option<usize> {
    marks
        .iter()
        .map(|mark| mark.frame)
        .filter(|&frame| frame > from && frame <= to)
        .min()
}

/// Frames kept around the reach onset (E2) and grip established (E4) events when clipping a trial.
pub const CLIP_MARGIN: usize = 10;

/// The inclusive frame range `E2 - 10 ..= E4 + 10` within a trial of `frame_count` frames. A missing event leaves
/// that side at the trial's edge.
pub fn clip_range(marks: &[GraspMark], frame_count: usize) -> (usize, usize) {
    let last = frame_count.saturating_sub(1);
    let frame_of = |kind| marks.iter().find(|mark| mark.kind == kind).map(|mark| mark.frame);
    let start = frame_of(GraspEventKind::ReachOnset).map_or(0, |f| f.saturating_sub(CLIP_MARGIN));
    let end = frame_of(GraspEventKind::GripEstablished).map_or(last, |f| (f + CLIP_MARGIN).min(last));
    if start > end { (0, last) } else { (start, end) }
}

/// The grasp events the pipeline recorded in an export's metadata (`grasp_events`).
#[derive(Debug, Clone, PartialEq, Default, Deserialize)]
#[serde(from = "RawGraspRow")]
pub struct GraspRow {
    pub left_used: Option<bool>,
    pub right_used: Option<bool>,
    pub events: [GraspEvent; 3],
}

#[derive(Default, Deserialize)]
struct RawEvent {
    time: Option<f64>,
    certainty: Option<String>,
}

#[derive(Default, Deserialize)]
struct RawGraspRow {
    #[serde(default)]
    e2: RawEvent,
    #[serde(default)]
    e3: RawEvent,
    #[serde(default)]
    e4: RawEvent,
    left_hand_used: Option<bool>,
    right_hand_used: Option<bool>,
}

impl From<RawGraspRow> for GraspRow {
    fn from(raw: RawGraspRow) -> Self {
        let event = |raw: RawEvent| GraspEvent {
            time: raw.time.filter(|seconds| seconds.is_finite() && *seconds >= 0.0),
            certainty: raw.certainty.as_deref().map(Certainty::parse),
        };
        Self {
            left_used: raw.left_hand_used,
            right_used: raw.right_hand_used,
            events: [event(raw.e2), event(raw.e3), event(raw.e4)],
        }
    }
}

/// `(left, right)`: whether a hand is not marked as used (false or unknown).
pub fn unused_sides(left_used: Option<bool>, right_used: Option<bool>) -> (bool, bool) {
    (left_used != Some(true), right_used != Some(true))
}

impl GraspRow {
    pub fn event(&self, kind: GraspEventKind) -> &GraspEvent {
        &self.events[kind as usize]
    }

    pub fn unused_sides(&self) -> (bool, bool) {
        unused_sides(self.left_used, self.right_used)
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
