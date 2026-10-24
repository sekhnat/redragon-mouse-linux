//! Change-set diffing between the device baseline and the pending draft.

use super::state::WriteOp;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OpChange {
    pub id: String,
    pub label: String,
    pub profile: Option<u8>,
    pub before: Option<Vec<u8>>,
    pub after: Vec<u8>,
}

/// Draft operations whose payload differs from the baseline (or that the
/// baseline does not contain at all).
pub fn changed_ops<'a>(baseline: &[WriteOp], draft: &'a [WriteOp]) -> Vec<&'a WriteOp> {
    draft
        .iter()
        .filter(|op| match baseline.iter().find(|base| base.id == op.id) {
            Some(base) => base.payload != op.payload,
            None => true,
        })
        .collect()
}

/// Human-readable change set for the confirmation dialog and logs.
pub fn summary(baseline: &[WriteOp], draft: &[WriteOp]) -> Vec<OpChange> {
    changed_ops(baseline, draft)
        .into_iter()
        .map(|op| OpChange {
            id: op.id.clone(),
            label: op.describe(),
            profile: op.profile,
            before: baseline
                .iter()
                .find(|base| base.id == op.id)
                .map(|base| base.payload.clone()),
            after: op.payload.clone(),
        })
        .collect()
}

pub fn describe_payload(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn op(id: &str, payload: &[u8]) -> WriteOp {
        WriteOp {
            id: id.into(),
            label: id.into(),
            profile: Some(0),
            report: 2,
            addr: 0x20,
            payload: payload.to_vec(),
        }
    }

    #[test]
    fn only_changed_registers_are_selected() {
        let baseline = vec![op("a", &[1]), op("b", &[2]), op("c", &[3])];
        let draft = vec![op("a", &[1]), op("b", &[9])];
        let changed = changed_ops(&baseline, &draft);
        assert_eq!(changed.len(), 1);
        assert_eq!(changed[0].id, "b");
    }

    #[test]
    fn unchanged_draft_produces_no_change_set() {
        let baseline = vec![op("a", &[1])];
        let draft = vec![op("a", &[1])];
        assert!(summary(&baseline, &draft).is_empty());
    }

    #[test]
    fn summary_reports_before_and_after() {
        let baseline = vec![op("lighting.brightness:0", &[2])];
        let draft = vec![op("lighting.brightness:0", &[1])];
        let changes = summary(&baseline, &draft);
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].before.as_deref(), Some(&[2u8][..]));
        assert_eq!(changes[0].after, vec![1]);
    }
}
