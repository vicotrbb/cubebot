use super::Mark;
use crate::meetings::live::wire::Stream;
use std::collections::{HashMap, HashSet};

const STEP: u64 = 500;
const HOLD: u64 = 1500;
const SHORTEST: u64 = 3000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Look {
    Camera,
    Screen,
    Desktop,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Visual {
    pub slot: u16,
    pub look: Look,
    pub file: String,
    pub from: u64,
    pub to: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Run {
    pub file: String,
    pub offset: u64,
    pub rate: u32,
    pub at: u64,
}

#[derive(Debug, Default)]
pub(super) struct Timeline {
    pub duration: u64,
    pub names: HashMap<u16, String>,
    pub visuals: Vec<Visual>,
    pub runs: Vec<Run>,
    loud: HashMap<u64, Vec<u16>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Focus {
    Visual(usize),
    Slate(Option<u16>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Piece {
    pub from: u64,
    pub to: u64,
    pub focus: Focus,
}

impl Timeline {
    pub(super) fn read(marks: impl IntoIterator<Item = Mark>) -> Self {
        let mut timeline = Self::default();
        let mut open: HashMap<String, usize> = HashMap::new();
        for mark in marks {
            timeline.duration = timeline.duration.max(mark.at());
            timeline.take(mark, &mut open);
        }
        for index in open.into_values() {
            timeline.visuals[index].to = timeline.duration;
        }
        timeline
    }

    fn take(&mut self, mark: Mark, open: &mut HashMap<String, usize>) {
        match mark {
            Mark::Name { slot, name, .. } => {
                self.names.insert(slot, name);
            }
            Mark::Voice {
                at,
                rate,
                file,
                offset,
                ..
            } => self.runs.push(Run {
                file,
                offset,
                rate,
                at,
            }),
            Mark::Loud { at, slot } => self.loud.entry(at / STEP * STEP).or_default().push(slot),
            Mark::Video {
                at,
                slot,
                stream,
                file,
            } => {
                let look = match stream {
                    Stream::Camera => Look::Camera,
                    Stream::Screen => Look::Screen,
                };
                open.insert(file.clone(), self.visuals.len());
                self.visuals.push(Visual {
                    slot,
                    look,
                    file,
                    from: at,
                    to: at,
                });
            }
            Mark::Desktop { at, slot, file } => {
                open.insert(file.clone(), self.visuals.len());
                let look = Look::Desktop;
                self.visuals.push(Visual {
                    slot,
                    look,
                    file,
                    from: at,
                    to: at,
                });
            }
            Mark::Stop { at, file } => {
                if let Some(index) = open.remove(&file) {
                    self.visuals[index].to = at;
                }
            }
            Mark::End { .. } => {}
        }
    }

    fn active(&self, index: usize, from: u64, to: u64) -> bool {
        let visual = &self.visuals[index];
        visual.from <= from && to <= visual.to
    }

    fn live(&self, index: usize, at: u64) -> bool {
        let visual = &self.visuals[index];
        visual.from <= at && at < visual.to
    }

    fn shared(&self, at: u64) -> Option<usize> {
        self.visuals
            .iter()
            .enumerate()
            .filter(|(index, visual)| visual.look != Look::Camera && self.live(*index, at))
            .max_by_key(|(_, visual)| visual.from)
            .map(|(index, _)| index)
    }

    fn camera(&self, at: u64, slot: Option<u16>) -> Option<usize> {
        self.visuals
            .iter()
            .enumerate()
            .filter(|(index, visual)| visual.look == Look::Camera && self.live(*index, at))
            .find(|(_, visual)| slot.is_none_or(|slot| visual.slot == slot))
            .map(|(index, _)| index)
    }

    pub(super) fn label(&self, focus: Focus) -> Option<String> {
        let slot = match focus {
            Focus::Visual(index) => Some(self.visuals[index].slot),
            Focus::Slate(slot) => slot,
        }?;
        let name = self.names.get(&slot)?.clone();
        let screen =
            matches!(focus, Focus::Visual(index) if self.visuals[index].look != Look::Camera);
        Some(if screen {
            format!("{name}'s screen")
        } else {
            name
        })
    }
}

#[derive(Default)]
struct Speaker {
    current: Option<u16>,
    heard: u64,
}

impl Speaker {
    fn follow(&mut self, at: u64, loud: Option<&Vec<u16>>) {
        let loud: HashSet<u16> = loud.into_iter().flatten().copied().collect();
        if self.current.is_some_and(|slot| loud.contains(&slot)) {
            self.heard = at;
            return;
        }
        let quiet = self.current.is_none() || at >= self.heard + HOLD;
        if let Some(&next) = loud.iter().min().filter(|_| quiet) {
            self.current = Some(next);
            self.heard = at;
        }
    }
}

fn focus_at(timeline: &Timeline, at: u64, speaker: Option<u16>) -> Focus {
    if let Some(index) = timeline.shared(at) {
        return Focus::Visual(index);
    }
    if let Some(index) = speaker.and_then(|slot| timeline.camera(at, Some(slot))) {
        return Focus::Visual(index);
    }
    if speaker.is_some() {
        return Focus::Slate(speaker);
    }
    timeline
        .camera(at, None)
        .map_or(Focus::Slate(None), Focus::Visual)
}

fn steps(timeline: &Timeline) -> Vec<Piece> {
    let mut speaker = Speaker::default();
    let mut pieces: Vec<Piece> = Vec::new();
    let mut at = 0;
    while at < timeline.duration {
        speaker.follow(at, timeline.loud.get(&at));
        let to = (at + STEP).min(timeline.duration);
        let focus = focus_at(timeline, (at + to) / 2, speaker.current);
        match pieces.last_mut() {
            Some(last) if last.focus == focus => last.to = to,
            _ => pieces.push(Piece {
                from: at,
                to,
                focus,
            }),
        }
        at = to;
    }
    pieces
}

fn holds(timeline: &Timeline, last: Focus, next: Focus, from: u64, to: u64) -> bool {
    match (last, next) {
        (Focus::Visual(index), _) => timeline.active(index, from, to),
        (Focus::Slate(_), Focus::Slate(_)) => true,
        (Focus::Slate(_), Focus::Visual(_)) => false,
    }
}

pub(super) fn plan(timeline: &Timeline) -> Vec<Piece> {
    let mut pieces: Vec<Piece> = Vec::new();
    for piece in steps(timeline) {
        let shared = timeline.shared(piece.from).is_some();
        match pieces.last_mut() {
            Some(last) if last.focus == piece.focus => last.to = piece.to,
            Some(last)
                if piece.to - piece.from < SHORTEST
                    && !shared
                    && holds(timeline, last.focus, piece.focus, piece.from, piece.to) =>
            {
                last.to = piece.to;
            }
            _ => pieces.push(piece),
        }
    }
    pieces
}

#[cfg(test)]
mod tests {
    use super::*;

    fn name(slot: u16, name: &str) -> Mark {
        Mark::Name {
            at: 0,
            slot,
            name: name.into(),
        }
    }

    fn camera(at: u64, slot: u16, file: &str) -> Mark {
        Mark::Video {
            at,
            slot,
            stream: Stream::Camera,
            file: file.into(),
        }
    }

    fn talk(slot: u16, from: u64, to: u64) -> Vec<Mark> {
        (from / STEP..to / STEP)
            .map(|step| Mark::Loud {
                at: step * STEP,
                slot,
            })
            .collect()
    }

    fn timeline(marks: Vec<Mark>, end: u64) -> Timeline {
        let mut marks = marks;
        marks.push(Mark::End { at: end });
        Timeline::read(marks)
    }

    fn shape(timeline: &Timeline) -> Vec<(u64, u64, Option<String>)> {
        plan(timeline)
            .into_iter()
            .map(|piece| (piece.from, piece.to, timeline.label(piece.focus)))
            .collect()
    }

    #[test]
    fn the_speaker_with_a_camera_fills_the_frame_and_one_without_gets_a_card() {
        let mut marks = vec![name(1, "Ana"), name(2, "Bo"), camera(0, 1, "c1-1.ivf")];
        marks.extend(talk(1, 0, 4000));
        marks.extend(talk(2, 6000, 10_000));
        let timeline = timeline(marks, 10_000);
        assert_eq!(
            shape(&timeline),
            [
                (0, 6000, Some("Ana".into())),
                (6000, 10_000, Some("Bo".into()))
            ]
        );
        assert_eq!(plan(&timeline)[0].focus, Focus::Visual(0));
        assert_eq!(plan(&timeline)[1].focus, Focus::Slate(Some(2)));
    }

    #[test]
    fn a_screen_share_wins_over_whoever_is_speaking_until_it_stops() {
        let mut marks = vec![
            name(1, "Ana"),
            name(2, "Bo"),
            camera(0, 1, "c1-1.ivf"),
            Mark::Video {
                at: 2000,
                slot: 2,
                stream: Stream::Screen,
                file: "s2-2.ivf".into(),
            },
            Mark::Stop {
                at: 8000,
                file: "s2-2.ivf".into(),
            },
        ];
        marks.extend(talk(1, 0, 12_000));
        let timeline = timeline(marks, 12_000);
        assert_eq!(
            shape(&timeline),
            [
                (0, 2000, Some("Ana".into())),
                (2000, 8000, Some("Bo's screen".into())),
                (8000, 12_000, Some("Ana".into()))
            ]
        );
    }

    #[test]
    fn a_short_interjection_does_not_cut_away_from_the_speaker() {
        let mut marks = vec![name(1, "Ana"), name(2, "Bo"), camera(0, 1, "c1-1.ivf")];
        marks.extend(talk(1, 0, 3000));
        marks.extend(talk(2, 5000, 5500));
        marks.extend(talk(1, 7000, 12_000));
        let timeline = timeline(marks, 12_000);
        assert_eq!(shape(&timeline), [(0, 12_000, Some("Ana".into()))]);
    }

    #[test]
    fn nobody_speaking_and_no_camera_is_a_title_card_and_open_files_end_with_the_recording() {
        let marks = vec![Mark::Desktop {
            at: 4000,
            slot: 3,
            file: "d3-1.tiles".into(),
        }];
        let timeline = timeline(marks, 9000);
        assert_eq!(timeline.visuals[0].to, 9000);
        let pieces = plan(&timeline);
        assert_eq!(pieces[0].focus, Focus::Slate(None));
        assert_eq!((pieces[1].from, pieces[1].to), (4000, 9000));
        assert_eq!(pieces[1].focus, Focus::Visual(0));
    }

    #[test]
    fn a_camera_that_starts_just_after_the_recording_is_never_hidden_behind_a_card() {
        let mut marks = vec![name(1, "Ana"), camera(1, 1, "c1-1.ivf")];
        marks.extend(talk(1, 0, 1400));
        let timeline = timeline(marks, 1400);
        let pieces = plan(&timeline);
        assert_eq!(pieces.len(), 1);
        assert_eq!(pieces[0].focus, Focus::Visual(0));
    }
}
