use crate::meetings::AgentMode;
use serde::Deserialize;
use std::collections::{BTreeMap, HashSet, VecDeque};
use std::time::Duration;
use tokio::time::Instant;
use uuid::Uuid;

const MERGE: Duration = Duration::from_millis(700);
const HOLD: Duration = Duration::from_secs(15);
const HANDOFFS: u8 = 2;
const PENDING: usize = 2;

pub(super) const CHOOSER: &str = "You decide whether one AI agent in a live meeting should answer what was just said. You have no tools and do no work yourself. Pick an agent only when it is addressed, when a question is put to the group that one agent is clearly suited to answer, or when the person is continuing an exchange with that agent. Otherwise no agent answers: people talking among themselves, small talk, thanks and statements need no answer. Reply with JSON only, one of: {\"agent\":\"<agent id>\"} or {\"agent\":null}.";

#[derive(Debug, PartialEq, Eq)]
pub(super) enum Decision {
    Named(Vec<Uuid>),
    Ask,
    Nobody,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Choice {
    agent: Option<Uuid>,
}

struct Seated {
    id: Uuid,
    name: Vec<String>,
    about: String,
    called: String,
    listening: bool,
}

struct Waiting {
    text: String,
    at: Instant,
}

struct Queued {
    agent: Uuid,
    turn: Uuid,
    asker: Option<u16>,
}

#[derive(Default)]
pub(super) struct Floor {
    agents: Vec<Seated>,
    waiting: BTreeMap<u16, Waiting>,
    queue: VecDeque<Queued>,
    handoffs: u8,
}

impl Floor {
    pub(super) fn seat(&mut self, agent: Uuid, name: &str, about: &str) {
        self.unseat(agent);
        self.agents.push(Seated {
            id: agent,
            name: words(name),
            about: about.to_owned(),
            called: name.to_owned(),
            listening: false,
        });
    }

    pub(super) fn set_listening(&mut self, agent: Uuid, listening: bool) {
        if let Some(seated) = self.agents.iter_mut().find(|seated| seated.id == agent) {
            seated.listening = listening;
        }
    }

    fn speaking(&self) -> impl Iterator<Item = &Seated> {
        self.agents.iter().filter(|seated| !seated.listening)
    }

    pub(super) fn unseat(&mut self, agent: Uuid) {
        self.agents.retain(|seated| seated.id != agent);
        self.queue.retain(|queued| queued.agent != agent);
    }

    pub(super) fn started(&mut self, slot: u16, now: Instant) {
        if let Some(waiting) = self.waiting.get_mut(&slot) {
            waiting.at = now + HOLD;
        }
    }

    pub(super) fn heard(&mut self, slot: u16, text: &str, now: Instant) {
        self.handoffs = 0;
        let waiting = self.waiting.entry(slot).or_insert(Waiting {
            text: String::new(),
            at: now,
        });
        if !waiting.text.is_empty() {
            waiting.text.push(' ');
        }
        waiting.text.push_str(text);
        waiting.at = now + MERGE;
    }

    pub(super) fn due_at(&self) -> Option<Instant> {
        self.waiting.values().map(|waiting| waiting.at).min()
    }

    pub(super) fn due(&mut self, now: Instant) -> Vec<(u16, String)> {
        let mut ready: Vec<(Instant, u16)> = self
            .waiting
            .iter()
            .filter(|(_, waiting)| waiting.at <= now)
            .map(|(&slot, waiting)| (waiting.at, slot))
            .collect();
        ready.sort_unstable();
        ready
            .into_iter()
            .filter_map(|(_, slot)| Some((slot, self.waiting.remove(&slot)?.text)))
            .collect()
    }

    pub(super) fn decide(&self, mode: AgentMode, text: &str) -> Decision {
        let named = self.named(text);
        if mode == AgentMode::Listen {
            Decision::Nobody
        } else if !named.is_empty() {
            Decision::Named(named)
        } else if mode == AgentMode::Relevant && self.speaking().next().is_some() {
            Decision::Ask
        } else {
            Decision::Nobody
        }
    }

    pub(super) fn named(&self, text: &str) -> Vec<Uuid> {
        let said = words(text);
        let mut taken = HashSet::new();
        let mut found = Vec::new();
        for seated in self.speaking().filter(|seated| seated.name.len() > 1) {
            if let Some(at) = said
                .windows(seated.name.len())
                .position(|window| window == seated.name.as_slice())
            {
                taken.extend(at..at + seated.name.len());
                found.push((at, seated.id));
            }
        }
        for seated in self.speaking() {
            let Some(first) = seated.name.first() else {
                continue;
            };
            let unnamed = found.iter().all(|(_, agent)| *agent != seated.id);
            let at = (0..said.len()).find(|at| !taken.contains(at) && said[*at] == *first);
            if let Some(at) = at.filter(|_| unnamed) {
                found.push((at, seated.id));
            }
        }
        found.sort_by_key(|(at, _)| *at);
        found.into_iter().map(|(_, agent)| agent).collect()
    }

    pub(super) fn handoff(&mut self, from: Uuid, text: &str) -> Vec<Uuid> {
        if self.handoffs >= HANDOFFS {
            return Vec::new();
        }
        let named: Vec<Uuid> = self
            .named(text)
            .into_iter()
            .filter(|agent| *agent != from)
            .collect();
        if !named.is_empty() {
            self.handoffs += 1;
        }
        named
    }

    pub(super) fn chosen(&self, reply: &str) -> Option<Uuid> {
        let start = reply.find('{')?;
        let end = reply.rfind('}')?;
        let choice: Choice = serde_json::from_str(reply.get(start..=end)?).ok()?;
        choice
            .agent
            .filter(|agent| self.speaking().any(|seated| seated.id == *agent))
    }

    pub(super) fn prompt(&self, lines: &[String]) -> String {
        let agents: Vec<String> = self
            .speaking()
            .map(|seated| format!("- id {}: {}. {}", seated.id, seated.called, seated.about))
            .collect();
        format!(
            "Agents in the meeting:\n{}\n\nThe meeting so far, oldest first; the last line was just said:\n{}",
            agents.join("\n"),
            lines.join("\n")
        )
    }

    pub(super) fn admits(&self, agent: Uuid, asker: Option<u16>) -> bool {
        let silent = self
            .agents
            .iter()
            .any(|seated| seated.id == agent && seated.listening);
        let mut pending = self.queue.iter().filter(|queued| queued.agent == agent);
        !silent && pending.clone().count() < PENDING && pending.all(|queued| queued.asker != asker)
    }

    pub(super) fn queue(&mut self, agent: Uuid, turn: Uuid, asker: Option<u16>) {
        self.queue.push_back(Queued { agent, turn, asker });
    }

    pub(super) fn front(&self) -> Option<(Uuid, Uuid)> {
        self.queue.front().map(|queued| (queued.agent, queued.turn))
    }

    pub(super) fn pop(&mut self) {
        self.queue.pop_front();
    }

    pub(super) fn waits(&self, agent: Uuid) -> bool {
        self.queue.iter().any(|queued| queued.agent == agent)
    }

    pub(super) fn hush(&mut self, agent: Uuid, speaking: bool) -> bool {
        let fronted = self.front().is_some_and(|(front, _)| front == agent);
        if speaking {
            self.interrupt(agent);
        } else {
            self.queue.retain(|queued| queued.agent != agent);
        }
        fronted && !speaking
    }

    pub(super) fn interrupt(&mut self, agent: Uuid) {
        let front = self.queue.pop_front();
        self.queue.retain(|queued| queued.agent != agent);
        if let Some(front) = front {
            self.queue.push_front(front);
        }
    }
}

fn words(text: &str) -> Vec<String> {
    text.split(|character: char| !(character.is_alphanumeric() || matches!(character, '\'' | '’')))
        .map(bare)
        .filter(|word| !word.is_empty())
        .collect()
}

fn bare(word: &str) -> String {
    let word = word.to_lowercase();
    let word = word.trim_matches(['\'', '’']);
    word.strip_suffix("'s")
        .or_else(|| word.strip_suffix("’s"))
        .unwrap_or(word)
        .to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    const CARA: Uuid = Uuid::from_u128(1);
    const DEX: Uuid = Uuid::from_u128(2);
    const ANN: Uuid = Uuid::from_u128(3);

    fn floor() -> Floor {
        let mut floor = Floor::default();
        floor.seat(CARA, "Cara", "Plans the roadmap.");
        floor.seat(DEX, "Dex Miller", "Handles the budget.");
        floor
    }

    #[test]
    fn an_agent_is_named_by_its_name_or_first_name_as_a_whole_word() {
        let floor = floor();
        assert_eq!(floor.named("cara, what do you think?"), [CARA]);
        assert_eq!(floor.named("What is CARA's view"), [CARA]);
        assert_eq!(floor.named("Over to you, Dex."), [DEX]);
        assert_eq!(floor.named("dex miller: numbers please"), [DEX]);
        assert!(floor.named("Caramel and dexterity").is_empty());
        assert!(floor.named("We need a plan").is_empty());
    }

    #[test]
    fn several_named_agents_answer_in_the_order_named() {
        let floor = floor();
        assert_eq!(floor.named("Dex, then Cara: thoughts?"), [DEX, CARA]);
        assert_eq!(floor.named("Cara and Dex! And Cara again"), [CARA, DEX]);
    }

    #[test]
    fn a_full_name_wins_its_words_over_a_shared_first_name() {
        let mut floor = floor();
        floor.seat(ANN, "Dex", "Another Dex.");
        assert_eq!(floor.named("Dex Miller, go ahead"), [DEX]);
        assert_eq!(floor.named("Dex?"), [DEX, ANN]);
    }

    #[test]
    fn an_utterance_waits_700_ms_and_the_same_speaker_merges_and_restarts_the_wait() {
        let mut floor = floor();
        let start = Instant::now();
        floor.heard(4, "Cara, the plan", start);
        assert!(floor.due(start + Duration::from_millis(699)).is_empty());
        floor.started(4, start + Duration::from_millis(500));
        assert!(floor.due(start + Duration::from_secs(5)).is_empty());
        floor.heard(4, "for next week?", start + Duration::from_secs(6));
        assert_eq!(floor.due_at(), Some(start + Duration::from_millis(6700)));
        assert!(floor.due(start + Duration::from_millis(6699)).is_empty());
        assert_eq!(
            floor.due(start + Duration::from_millis(6700)),
            [(4, "Cara, the plan for next week?".to_owned())]
        );
        assert_eq!(floor.due_at(), None);
    }

    #[test]
    fn different_speakers_wait_apart_and_are_released_in_order() {
        let mut floor = floor();
        let start = Instant::now();
        floor.heard(5, "second", start + Duration::from_millis(100));
        floor.heard(4, "first", start);
        floor.started(9, start);
        let due = floor.due(start + Duration::from_secs(1));
        assert_eq!(due, [(4, "first".to_owned()), (5, "second".to_owned())]);
    }

    #[test]
    fn one_agent_speaks_at_a_time_and_the_others_queue() {
        let mut floor = floor();
        let (first, second) = (Uuid::from_u128(10), Uuid::from_u128(11));
        floor.queue(DEX, first, Some(4));
        floor.queue(CARA, second, Some(4));
        assert_eq!(floor.front(), Some((DEX, first)));
        assert!(floor.waits(CARA));
        floor.pop();
        assert_eq!(floor.front(), Some((CARA, second)));
        assert!(!floor.waits(DEX));
        floor.unseat(CARA);
        assert_eq!(floor.front(), None);
    }

    #[test]
    fn an_agent_holds_at_most_two_pending_turns_and_one_per_asker() {
        let mut floor = floor();
        let turn = |n: u128| Uuid::from_u128(20 + n);
        assert!(floor.admits(CARA, Some(4)));
        floor.queue(CARA, turn(0), Some(4));
        assert!(!floor.admits(CARA, Some(4)), "one pending turn per asker");
        assert!(floor.admits(DEX, Some(4)));
        assert!(floor.admits(CARA, None));
        floor.queue(CARA, turn(1), None);
        assert!(!floor.admits(CARA, Some(5)), "two pending turns per agent");
        floor.pop();
        assert!(floor.admits(CARA, Some(4)));
    }

    #[test]
    fn a_person_speaking_drops_the_speaking_agents_queued_replies_only() {
        let mut floor = floor();
        let turn = |n: u128| Uuid::from_u128(30 + n);
        floor.queue(CARA, turn(0), Some(4));
        floor.queue(DEX, turn(1), Some(4));
        floor.queue(CARA, turn(2), Some(5));
        floor.interrupt(CARA);
        assert_eq!(
            floor.front(),
            Some((CARA, turn(0))),
            "the answering turn stays"
        );
        floor.pop();
        assert_eq!(floor.front(), Some((DEX, turn(1))));
        floor.pop();
        assert_eq!(floor.front(), None);
    }

    #[test]
    fn agents_hand_the_floor_on_at_most_twice_before_a_person_speaks() {
        let mut floor = floor();
        assert_eq!(
            floor.handoff(CARA, "I am Cara and I agree"),
            Vec::<Uuid>::new()
        );
        assert_eq!(floor.handoff(CARA, "Dex, your numbers?"), [DEX]);
        assert_eq!(floor.handoff(DEX, "Cara, over to you"), [CARA]);
        assert!(floor.handoff(CARA, "Dex, again?").is_empty());
        floor.heard(1, "thanks", Instant::now());
        assert_eq!(floor.handoff(CARA, "Dex, again?"), [DEX]);
    }

    #[test]
    fn only_when_named_never_asks_the_model() {
        let floor = floor();
        assert_eq!(
            floor.decide(AgentMode::Named, "What should we do?"),
            Decision::Nobody
        );
        assert_eq!(
            floor.decide(AgentMode::Relevant, "What should we do?"),
            Decision::Ask
        );
        assert_eq!(
            floor.decide(AgentMode::Named, "Cara, what should we do?"),
            Decision::Named(vec![CARA])
        );
        assert_eq!(
            Floor::default().decide(AgentMode::Relevant, "Anyone?"),
            Decision::Nobody
        );
    }

    #[test]
    fn listen_only_never_answers_even_when_named() {
        let floor = floor();
        assert_eq!(
            floor.decide(AgentMode::Listen, "Cara, what should we do?"),
            Decision::Nobody
        );
        assert_eq!(floor.decide(AgentMode::Listen, "Anyone?"), Decision::Nobody);
    }

    #[test]
    fn a_listening_agent_is_skipped_when_named_and_in_handoffs() {
        let mut floor = floor();
        floor.set_listening(CARA, true);
        assert_eq!(
            floor.decide(AgentMode::Named, "Cara and Dex, thoughts?"),
            Decision::Named(vec![DEX])
        );
        assert_eq!(floor.decide(AgentMode::Named, "Cara?"), Decision::Nobody);
        assert!(floor.handoff(DEX, "Over to you, Cara").is_empty());
        assert_eq!(floor.chosen(&format!(r#"{{"agent":"{CARA}"}}"#)), None);
        assert!(!floor.prompt(&[]).contains("Cara"));
    }

    #[test]
    fn relevant_mode_with_only_listening_agents_asks_nobody() {
        let mut floor = floor();
        floor.set_listening(CARA, true);
        floor.set_listening(DEX, true);
        assert_eq!(
            floor.decide(AgentMode::Relevant, "What now?"),
            Decision::Nobody
        );
    }

    #[test]
    fn a_reply_queued_for_an_agent_that_starts_listening_is_dropped_and_none_is_admitted() {
        let mut floor = floor();
        let turn = |n: u128| Uuid::from_u128(40 + n);
        floor.queue(CARA, turn(0), Some(4));
        floor.queue(DEX, turn(1), Some(4));
        floor.queue(CARA, turn(2), Some(5));
        floor.set_listening(CARA, true);
        assert!(floor.hush(CARA, false));
        assert_eq!(floor.front(), Some((DEX, turn(1))));
        floor.pop();
        assert_eq!(floor.front(), None);
        assert!(!floor.admits(CARA, Some(6)));
        assert!(floor.admits(DEX, Some(6)));
    }

    #[test]
    fn an_agent_already_speaking_keeps_its_answering_turn_and_loses_the_rest() {
        let mut floor = floor();
        let turn = |n: u128| Uuid::from_u128(50 + n);
        floor.queue(CARA, turn(0), Some(4));
        floor.queue(CARA, turn(1), Some(5));
        assert!(!floor.hush(CARA, true));
        assert_eq!(floor.front(), Some((CARA, turn(0))));
        floor.pop();
        assert_eq!(floor.front(), None);
    }

    #[test]
    fn hushing_an_agent_queued_behind_the_front_one_leaves_the_front_alone() {
        let mut floor = floor();
        let turn = |n: u128| Uuid::from_u128(60 + n);
        floor.queue(DEX, turn(0), Some(4));
        floor.queue(CARA, turn(1), Some(4));
        assert!(!floor.hush(CARA, false));
        assert_eq!(floor.front(), Some((DEX, turn(0))));
        assert!(!floor.waits(CARA));
    }

    #[test]
    fn the_model_names_a_seated_agent_or_no_one_and_anything_else_means_no_one() {
        let floor = floor();
        assert_eq!(
            floor.chosen(&format!(r#"{{"agent":"{CARA}"}}"#)),
            Some(CARA)
        );
        assert_eq!(
            floor.chosen(&format!("```json\n{{\"agent\": \"{DEX}\"}}\n```")),
            Some(DEX)
        );
        assert_eq!(floor.chosen(r#"{"agent":null}"#), None);
        assert_eq!(floor.chosen(&format!(r#"{{"agent":"{ANN}"}}"#)), None);
        assert_eq!(floor.chosen(r#"{"agent":"cara"}"#), None);
        assert_eq!(
            floor.chosen(&format!(r#"{{"agent":"{CARA}","why":"x"}}"#)),
            None
        );
        assert_eq!(floor.chosen("Cara should answer"), None);
    }

    #[test]
    fn the_model_reads_the_agents_and_the_last_lines() {
        let prompt = floor().prompt(&["Ana: budget?".to_owned()]);
        assert!(prompt.contains(&format!("- id {DEX}: Dex Miller. Handles the budget.")));
        assert!(prompt.ends_with("just said:\nAna: budget?"), "{prompt}");
    }
}
