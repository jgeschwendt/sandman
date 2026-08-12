//! 2-of-3 — the mechanical half of the dream.
//!
//! Three minds read one conversation and each proposes memories. This module
//! decides which of those proposals are *the same claim*, and nothing else:
//! no model is asked, no wording is rewritten, and the same proposals in any
//! order give the same groups. A hallucinated memory dies here, because one
//! witness is not enough.
//!
//! Sameness is measured on tokens drawn from the name *and* the description:
//! each field goes through the M1 slug rule and is split on `_`, every token of
//! four characters or more loses one trailing `s`, and the two lists are
//! unioned into one set. Two proposals in the same bank and of the same type
//! are the same claim when their token sets overlap by at least half
//! (Jaccard ≥ ½, compared as `2·|∩| ≥ |∪|` so no float ever decides a commit).
//!
//! The name alone was too narrow. In the first real dream three minds proposed
//! one fact as `deploy-thursdays-never-fridays`, `smoke-deploy-thursdays` and
//! `deploy-cadence-thursday`: pairwise name overlap sits under ½, so three
//! witnesses became three one-vote groups and a true memory was dropped. Minds
//! disagree about how to abbreviate a claim into a name and agree about how to
//! state it in a sentence, so the description is where the shared wording
//! actually lives — and cutting one trailing `s` costs nothing to stop
//! `thursdays` and `thursday` reading as two different words.
//!
//! The stemming is deliberately crude: one `s`, no exceptions, so `class`
//! becomes `clas`. It only has to be *the same* for every mind, not right.
//! Each field is slugged on its own, which also means each is cut at
//! [`slug::SLUG_MAX_CHARS`] — a long description contributes only its first
//! sixty characters.

use crate::memory::MemoryType;
use crate::mind::Tier;
use crate::slug;

/// The overlap two proposals need to be read as one claim, as a fraction.
/// Kept as a pair so the comparison stays integer arithmetic.
pub const AGREEMENT_NUMERATOR: usize = 1;
/// The denominator of [`AGREEMENT_NUMERATOR`].
pub const AGREEMENT_DENOMINATOR: usize = 2;
/// How many distinct minds a group needs before it commits.
pub const QUORUM: usize = 2;
/// Tokens at least this long lose one trailing `s`. Shorter ones are left
/// alone, so `abs` stays `abs` while `bugs` becomes `bug`.
pub const STEM_MIN_CHARS: usize = 4;

/// One memory a mind proposed.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Proposal {
    /// The bank key it goes to.
    pub bank: String,
    /// The body — the claim, in markdown.
    pub body: String,
    /// The one-line description. Its tokens count toward agreement too.
    pub description: String,
    /// The memory type.
    pub kind: MemoryType,
    /// The memory's name; its tokens and the description's are what agreement
    /// is measured on.
    pub name: String,
}

/// A claim, and the minds that made it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Group {
    /// The wording that carries: the draft from the strongest tier here.
    pub draft: Proposal,
    /// The distinct minds that proposed this claim, weakest first.
    pub tiers: Vec<Tier>,
}

impl Group {
    /// Whether enough minds witnessed this claim for it to commit.
    #[must_use]
    pub fn agreed(&self) -> bool {
        self.tiers.len() >= QUORUM
    }

    /// The minds, for a log line and for `source:`.
    #[must_use]
    pub fn minds(&self) -> String {
        self.tiers
            .iter()
            .map(|tier| tier.as_str())
            .collect::<Vec<_>>()
            .join(",")
    }
}

/// Group the proposals by claim.
///
/// Deterministic by construction: the voices are first put in a total order
/// that does not depend on how they arrived (strongest tier first, then the
/// proposal's own text), and grouping is then a single greedy pass. Strongest
/// first is also what makes the first member of a group its draft.
#[must_use]
pub fn group(voices: Vec<(Tier, Proposal)>) -> Vec<Group> {
    let mut voices: Vec<Voice> = voices
        .into_iter()
        .map(|(tier, proposal)| Voice {
            tokens: claim_tokens(&proposal.name, &proposal.description),
            proposal,
            tier,
        })
        .collect();
    voices.sort_by(|left, right| {
        right
            .tier
            .cmp(&left.tier)
            .then_with(|| left.proposal.bank.cmp(&right.proposal.bank))
            .then_with(|| left.proposal.kind.cmp(&right.proposal.kind))
            .then_with(|| left.tokens.cmp(&right.tokens))
            .then_with(|| left.proposal.name.cmp(&right.proposal.name))
            .then_with(|| left.proposal.description.cmp(&right.proposal.description))
            .then_with(|| left.proposal.body.cmp(&right.proposal.body))
    });

    let mut groups: Vec<Building> = Vec::new();
    for voice in voices {
        let joined = groups.iter_mut().find(|group| {
            group.draft.bank == voice.proposal.bank
                && group.draft.kind == voice.proposal.kind
                && same_claim(&group.tokens, &voice.tokens)
        });
        match joined {
            Some(group) => {
                if !group.tiers.contains(&voice.tier) {
                    group.tiers.push(voice.tier);
                }
            }
            None => groups.push(Building {
                draft: voice.proposal,
                tiers: vec![voice.tier],
                tokens: voice.tokens,
            }),
        }
    }

    groups
        .into_iter()
        .map(|mut group| {
            group.tiers.sort_unstable();
            Group {
                draft: group.draft,
                tiers: group.tiers,
            }
        })
        .collect()
}

/// One proposal from one mind, with its tokens precomputed.
struct Voice {
    /// What was proposed.
    proposal: Proposal,
    /// Which mind proposed it.
    tier: Tier,
    /// The name's and description's tokens, sorted and deduplicated.
    tokens: Vec<String>,
}

/// A group under construction: its draft is the first voice that landed in it,
/// which is the strongest tier's by the sort above.
struct Building {
    /// The wording that carries.
    draft: Proposal,
    /// The distinct minds so far.
    tiers: Vec<Tier>,
    /// The draft's tokens — every later voice is compared against these, so
    /// membership never depends on the order the rest arrived in.
    tokens: Vec<String>,
}

/// One field's tokens, as a set: the slug rule split on `_`, depluralized,
/// then sorted, deduplicated, empties dropped. Stemming runs before the dedupe,
/// so `bug` and `bugs` in one field collapse to a single token.
#[must_use]
pub fn tokens(text: &str) -> Vec<String> {
    let mut tokens: Vec<String> = slug::slug(text)
        .split('_')
        .filter(|token| !token.is_empty())
        .map(depluralize)
        .collect();
    tokens.sort();
    tokens.dedup();
    tokens
}

/// What agreement is measured on: the union of the name's tokens and the
/// description's. See the module docs for why the description is in here.
#[must_use]
pub fn claim_tokens(name: &str, description: &str) -> Vec<String> {
    let mut tokens = tokens(name);
    tokens.extend(self::tokens(description));
    tokens.sort();
    tokens.dedup();
    tokens
}

/// One trailing `s` off any token of [`STEM_MIN_CHARS`] or more. Crude on
/// purpose — it has to be identical across minds, not correct English.
fn depluralize(token: &str) -> String {
    // Slug output is ASCII, so a byte cut is a character cut.
    if token.len() >= STEM_MIN_CHARS && token.ends_with('s') {
        token[..token.len() - 1].to_owned()
    } else {
        token.to_owned()
    }
}

/// Whether two token sets overlap by at least [`AGREEMENT_NUMERATOR`] /
/// [`AGREEMENT_DENOMINATOR`]. Two empty sets agree about nothing.
#[must_use]
pub fn same_claim(left: &[String], right: &[String]) -> bool {
    if left.is_empty() || right.is_empty() {
        return false;
    }
    let shared = left.iter().filter(|token| right.contains(token)).count();
    let union = left.len() + right.len() - shared;
    shared * AGREEMENT_DENOMINATOR >= union * AGREEMENT_NUMERATOR
}

#[cfg(test)]
mod tests {
    use super::{Group, Proposal, QUORUM, claim_tokens, group, same_claim, tokens};
    use crate::memory::MemoryType;
    use crate::mind::Tier;

    const BANK: &str = "-Users-you-code";

    fn proposal(name: &str, body: &str) -> Proposal {
        Proposal {
            bank: BANK.to_owned(),
            body: body.to_owned(),
            description: format!("description of {name}"),
            kind: MemoryType::Feedback,
            name: name.to_owned(),
        }
    }

    /// A proposal whose description is written out rather than derived.
    fn described(name: &str, description: &str, body: &str) -> Proposal {
        Proposal {
            description: description.to_owned(),
            ..proposal(name, body)
        }
    }

    fn committed(groups: &[Group]) -> Vec<&Group> {
        groups.iter().filter(|group| group.agreed()).collect()
    }

    #[test]
    fn the_token_set_is_the_slug_split_on_underscores_and_depluralized() {
        // Changed: `hooks` used to survive as `hooks`; tokens are now stemmed.
        assert_eq!(tokens("Hooks are disabled"), ["are", "disabled", "hook"]);
        // Repeats collapse: a set, not a bag.
        assert_eq!(tokens("cache the cache"), ["cache", "the"]);
        assert!(tokens("———").is_empty());
    }

    #[test]
    fn the_trailing_s_comes_off_at_four_characters() {
        assert_eq!(tokens("abs bugs"), ["abs", "bug"]);
        // The boundary is the token's length before the cut: `ads` is three
        // characters and keeps its `s`, `bugs` is four and loses it.
        assert_eq!(tokens("ads"), ["ads"]);
        assert_eq!(tokens("bugs"), ["bug"]);
        // Singular and plural of one word collapse into one token.
        assert_eq!(tokens("a bug and two bugs"), ["a", "and", "bug", "two"]);
        // Crude on purpose: it is consistent, not correct.
        assert_eq!(tokens("class status"), ["clas", "statu"]);
    }

    #[test]
    fn a_claim_is_tokenized_over_the_name_and_the_description() {
        assert_eq!(
            claim_tokens("smoke deploys", "smoke deploys ship Thursday"),
            ["deploy", "ship", "smoke", "thursday"]
        );
        // The description alone is enough to have tokens to agree on.
        assert_eq!(claim_tokens("———", "ship Thursday"), ["ship", "thursday"]);
    }

    #[test]
    fn agreement_is_jaccard_at_one_half_inclusive() {
        let set = |names: &[&str]| -> Vec<String> {
            let mut set: Vec<String> = names.iter().map(|name| (*name).to_owned()).collect();
            set.sort();
            set
        };
        // 3 shared of 5 united → 0.6.
        assert!(same_claim(
            &set(&["a", "b", "c", "d"]),
            &set(&["a", "b", "c", "e"])
        ));
        // 2 shared of 4 united → exactly 0.5, which agrees.
        assert!(same_claim(&set(&["a", "b"]), &set(&["a", "b", "c", "d"])));
        // 2 shared of 5 united → 0.4, which does not.
        assert!(!same_claim(
            &set(&["a", "b"]),
            &set(&["a", "b", "c", "d", "e"])
        ));
        // 1 shared of 3 united → 0.33.
        assert!(!same_claim(&set(&["a", "b"]), &set(&["a", "c"])));
        assert!(!same_claim(&set(&["a"]), &set(&[])));
        assert!(!same_claim(&set(&[]), &set(&[])));
        // Identical sets always agree.
        assert!(same_claim(&set(&["a"]), &set(&["a"])));
    }

    #[test]
    fn two_of_three_commits_and_the_lone_mind_is_dropped() {
        let groups = group(vec![
            (
                Tier::Sonnet,
                proposal("the queue is the recall surface", "sonnet's wording"),
            ),
            (
                Tier::Opus,
                proposal("the queue is the recall surface", "opus's wording"),
            ),
            (Tier::Fable, proposal("something else entirely", "alone")),
        ]);
        assert_eq!(groups.len(), 2);
        let agreed = committed(&groups);
        assert_eq!(agreed.len(), 1);
        assert_eq!(agreed[0].tiers, [Tier::Sonnet, Tier::Opus]);
        assert_eq!(agreed[0].minds(), "sonnet,opus");
        // The strongest tier in the group carries the wording.
        assert_eq!(agreed[0].draft.body, "opus's wording");
        let alone: Vec<&Group> = groups.iter().filter(|group| !group.agreed()).collect();
        assert_eq!(alone.len(), 1);
        assert_eq!(alone[0].tiers, [Tier::Fable]);
    }

    #[test]
    fn the_strongest_agreeing_tier_carries_the_wording() {
        let groups = group(vec![
            (
                Tier::Sonnet,
                proposal("hooks disabled on the work account", "s"),
            ),
            (
                Tier::Fable,
                proposal("hooks are disabled work account", "f"),
            ),
            (Tier::Opus, proposal("hooks disabled account work", "o")),
        ]);
        assert_eq!(groups.len(), 1);
        assert!(groups[0].agreed());
        assert_eq!(groups[0].draft.body, "f");
        assert_eq!(groups[0].minds(), "sonnet,opus,fable");
    }

    #[test]
    fn grouping_does_not_depend_on_the_order_the_minds_answered_in() {
        let voices = vec![
            (Tier::Sonnet, proposal("the queue is the surface", "s")),
            (Tier::Opus, proposal("the queue is a surface", "o")),
            (Tier::Fable, proposal("stele owns the graph lock", "f")),
            (Tier::Sonnet, proposal("stele owns the graph lock", "s2")),
        ];
        let expected = group(voices.clone());
        // Every permutation of four voices.
        let mut permutations = 0;
        for a in 0..4 {
            for b in 0..4 {
                for c in 0..4 {
                    for d in 0..4 {
                        let indexes = [a, b, c, d];
                        let mut seen = indexes;
                        seen.sort_unstable();
                        if seen != [0, 1, 2, 3] {
                            continue;
                        }
                        permutations += 1;
                        let shuffled: Vec<_> =
                            indexes.iter().map(|index| voices[*index].clone()).collect();
                        assert_eq!(group(shuffled), expected, "for {indexes:?}");
                    }
                }
            }
        }
        assert_eq!(permutations, 24);
        assert_eq!(expected.len(), 2);
        assert!(expected.iter().all(Group::agreed));
    }

    #[test]
    fn a_claim_never_crosses_a_bank_or_a_type() {
        let mut elsewhere = proposal("the queue is the surface", "elsewhere");
        elsewhere.bank = "-Users-you".to_owned();
        let mut retyped = proposal("the queue is the surface", "retyped");
        retyped.kind = MemoryType::Project;
        let groups = group(vec![
            (Tier::Sonnet, proposal("the queue is the surface", "here")),
            (Tier::Opus, elsewhere),
            (Tier::Fable, retyped),
        ]);
        assert_eq!(groups.len(), 3);
        assert!(committed(&groups).is_empty());
    }

    #[test]
    fn one_mind_proposing_twice_is_still_one_witness() {
        let groups = group(vec![
            (Tier::Sonnet, proposal("the queue is the surface", "first")),
            (Tier::Sonnet, proposal("the queue is a surface", "second")),
        ]);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].tiers, [Tier::Sonnet]);
        assert!(!groups[0].agreed());
    }

    #[test]
    fn nothing_proposed_is_nothing_grouped() {
        assert!(group(Vec::new()).is_empty());
    }

    /// The first real dream: three minds, one fact about the deploy day, three
    /// names too far apart to meet on the names alone.
    fn the_deploy_day() -> Vec<(Tier, Proposal)> {
        vec![
            (
                Tier::Sonnet,
                described(
                    "deploy-thursdays-never-fridays",
                    "deploys ship Thursday, never Friday",
                    "s",
                ),
            ),
            (
                Tier::Opus,
                described(
                    "smoke-deploy-thursdays",
                    "smoke deploys ship Thursday, never Friday",
                    "o",
                ),
            ),
            (
                Tier::Fable,
                described(
                    "deploy-cadence-thursday",
                    "the deploy cadence: ship Thursday, never Friday",
                    "f",
                ),
            ),
        ]
    }

    #[test]
    fn the_live_trio_that_the_name_only_rule_split_now_reaches_quorum() {
        // The rule that dropped this memory: the name's slug tokens, unstemmed.
        // No pair of the three reaches ½ under it, so three witnesses became
        // three one-vote groups.
        let old_rule = |name: &str| -> Vec<String> {
            let mut tokens: Vec<String> = crate::slug::slug(name)
                .split('_')
                .filter(|token| !token.is_empty())
                .map(ToOwned::to_owned)
                .collect();
            tokens.sort();
            tokens.dedup();
            tokens
        };
        let names: Vec<Vec<String>> = the_deploy_day()
            .iter()
            .map(|(_, proposal)| old_rule(&proposal.name))
            .collect();
        assert!(!same_claim(&names[0], &names[1]));
        assert!(!same_claim(&names[0], &names[2]));
        assert!(!same_claim(&names[1], &names[2]));

        let groups = group(the_deploy_day());
        let agreed = committed(&groups);
        assert_eq!(agreed.len(), 1, "one claim should commit, got {groups:?}");
        assert!(
            agreed[0].tiers.len() >= QUORUM,
            "at least two of the three minds must land together"
        );
        // In fact all three do, and the strongest tier carries the wording.
        assert_eq!(agreed[0].tiers, [Tier::Sonnet, Tier::Opus, Tier::Fable]);
        assert_eq!(agreed[0].draft.body, "f");
    }

    #[test]
    fn the_widened_tokens_are_still_order_independent() {
        let mut voices = the_deploy_day();
        voices.push((
            Tier::Opus,
            described(
                "hooks-disabled-on-the-work-account",
                "hooks stay disabled on the work account",
                "unrelated",
            ),
        ));
        let expected = group(voices.clone());
        assert_eq!(expected.len(), 2);
        assert_eq!(committed(&expected).len(), 1);

        let mut permutations = 0;
        for a in 0..4 {
            for b in 0..4 {
                for c in 0..4 {
                    for d in 0..4 {
                        let indexes = [a, b, c, d];
                        let mut seen = indexes;
                        seen.sort_unstable();
                        if seen != [0, 1, 2, 3] {
                            continue;
                        }
                        permutations += 1;
                        let shuffled: Vec<_> =
                            indexes.iter().map(|index| voices[*index].clone()).collect();
                        assert_eq!(group(shuffled), expected, "for {indexes:?}");
                    }
                }
            }
        }
        assert_eq!(permutations, 24);
    }
}
