use crate::utils::{color::Palette, contract_resolver::ContractResolver};
use serde::Deserialize;
use std::collections::{BTreeMap, HashMap, HashSet};

/// Prestate tracer result in `diffMode: true`.
///
/// Each map entry is keyed by contract address. `pre` captures state before the
/// transaction; `post` captures state after. Only modified accounts are included.
///
/// Both fields are always present in valid diffMode output. A missing key causes
/// deserialization to fail, catching non-diffMode responses early.
#[derive(Debug, Deserialize)]
pub struct PrestateDiff {
    pub pre: HashMap<String, AccountState>,
    pub post: HashMap<String, AccountState>,
}

#[derive(Debug, Deserialize, Default)]
pub struct AccountState {
    #[serde(default)]
    pub storage: HashMap<String, String>,
}

struct SlotChange {
    slot: String,
    before: Option<String>,
    after: Option<String>,
}

/// Compute per-address storage slot changes from a prestate diff.
fn compute_changes(diff: &PrestateDiff) -> BTreeMap<&str, Vec<SlotChange>> {
    let mut changes_by_addr: BTreeMap<&str, Vec<SlotChange>> = BTreeMap::new();

    let all_addrs: HashSet<&str> = diff
        .pre
        .keys()
        .chain(diff.post.keys())
        .map(String::as_str)
        .collect();

    for addr in all_addrs {
        let pre_storage = diff.pre.get(addr).map(|a| &a.storage);
        let post_storage = diff.post.get(addr).map(|a| &a.storage);

        let mut slots = Vec::new();

        if let Some(pre_s) = pre_storage {
            for (slot, pre_val) in pre_s {
                match post_storage.and_then(|ps| ps.get(slot)) {
                    Some(post_val) if post_val != pre_val => {
                        slots.push(SlotChange {
                            slot: slot.clone(),
                            before: Some(pre_val.clone()),
                            after: Some(post_val.clone()),
                        });
                    }
                    None => {
                        slots.push(SlotChange {
                            slot: slot.clone(),
                            before: Some(pre_val.clone()),
                            after: None,
                        });
                    }
                    _ => {}
                }
            }
        }

        if let Some(post_s) = post_storage {
            for (slot, post_val) in post_s {
                let in_pre = pre_storage.is_some_and(|ps| ps.contains_key(slot));
                if !in_pre {
                    slots.push(SlotChange {
                        slot: slot.clone(),
                        before: None,
                        after: Some(post_val.clone()),
                    });
                }
            }
        }

        if !slots.is_empty() {
            slots.sort_by(|a, b| a.slot.cmp(&b.slot));
            changes_by_addr.insert(addr, slots);
        }
    }

    changes_by_addr
}

/// Print storage slot diffs grouped by contract address.
pub fn print_storage_diff(
    diff: &PrestateDiff,
    contract_resolver: &mut ContractResolver,
    pal: Palette,
) {
    let changes_by_addr = compute_changes(diff);

    if changes_by_addr.is_empty() {
        return;
    }

    println!();
    println!("Storage changes:");
    for (addr, slots) in &changes_by_addr {
        let header = match contract_resolver.resolve(addr) {
            Some(name) => format!("  {} ({}):", pal.cyan(&name), pal.dim(addr)),
            None => format!("  {}:", pal.cyan(addr)),
        };
        println!("{header}");

        for change in slots {
            let before = change.before.as_deref().unwrap_or("0x0");
            let after = change.after.as_deref().unwrap_or("0x0");
            println!(
                "    {} {} {} {}",
                pal.dim(&format!("@ {}:", change.slot)),
                before,
                pal.dim("→"),
                after,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_diff(
        pre: Vec<(&str, Vec<(&str, &str)>)>,
        post: Vec<(&str, Vec<(&str, &str)>)>,
    ) -> PrestateDiff {
        let to_map = |entries: Vec<(&str, Vec<(&str, &str)>)>| {
            entries
                .into_iter()
                .map(|(addr, slots)| {
                    let storage = slots
                        .into_iter()
                        .map(|(k, v)| (k.to_string(), v.to_string()))
                        .collect();
                    (addr.to_string(), AccountState { storage })
                })
                .collect()
        };
        PrestateDiff {
            pre: to_map(pre),
            post: to_map(post),
        }
    }

    #[test]
    fn test_compute_changes_slot_modified() {
        let diff = make_diff(
            vec![("0xaaa", vec![("0x01", "0xold")])],
            vec![("0xaaa", vec![("0x01", "0xnew")])],
        );
        let changes = compute_changes(&diff);
        assert_eq!(changes.len(), 1);
        let slots = &changes["0xaaa"];
        assert_eq!(slots.len(), 1);
        assert_eq!(slots[0].slot, "0x01");
        assert_eq!(slots[0].before.as_deref(), Some("0xold"));
        assert_eq!(slots[0].after.as_deref(), Some("0xnew"));
    }

    #[test]
    fn test_compute_changes_slot_unchanged() {
        let diff = make_diff(
            vec![("0xaaa", vec![("0x01", "0xsame")])],
            vec![("0xaaa", vec![("0x01", "0xsame")])],
        );
        let changes = compute_changes(&diff);
        assert!(changes.is_empty());
    }

    #[test]
    fn test_compute_changes_slot_new() {
        let diff = make_diff(vec![], vec![("0xaaa", vec![("0x01", "0xnew")])]);
        let changes = compute_changes(&diff);
        assert_eq!(changes.len(), 1);
        let slots = &changes["0xaaa"];
        assert_eq!(slots.len(), 1);
        assert_eq!(slots[0].before, None);
        assert_eq!(slots[0].after.as_deref(), Some("0xnew"));
    }

    #[test]
    fn test_compute_changes_slot_deleted() {
        let diff = make_diff(vec![("0xaaa", vec![("0x01", "0xold")])], vec![]);
        let changes = compute_changes(&diff);
        assert_eq!(changes.len(), 1);
        let slots = &changes["0xaaa"];
        assert_eq!(slots.len(), 1);
        assert_eq!(slots[0].before.as_deref(), Some("0xold"));
        assert_eq!(slots[0].after, None);
    }

    #[test]
    fn test_compute_changes_empty_diff() {
        let diff = make_diff(vec![], vec![]);
        let changes = compute_changes(&diff);
        assert!(changes.is_empty());
    }

    #[test]
    fn test_compute_changes_addresses_sorted() {
        let diff = make_diff(
            vec![
                ("0xzzz", vec![("0x01", "0xa")]),
                ("0xaaa", vec![("0x01", "0xb")]),
            ],
            vec![
                ("0xzzz", vec![("0x01", "0xc")]),
                ("0xaaa", vec![("0x01", "0xd")]),
            ],
        );
        let changes = compute_changes(&diff);
        let addrs: Vec<&&str> = changes.keys().collect();
        assert_eq!(addrs, vec![&"0xaaa", &"0xzzz"]);
    }

    #[test]
    fn test_compute_changes_slots_sorted() {
        let diff = make_diff(
            vec![("0xaaa", vec![("0x03", "0xa"), ("0x01", "0xb")])],
            vec![("0xaaa", vec![("0x03", "0xc"), ("0x01", "0xd")])],
        );
        let changes = compute_changes(&diff);
        let slots: Vec<&str> = changes["0xaaa"].iter().map(|s| s.slot.as_str()).collect();
        assert_eq!(slots, vec!["0x01", "0x03"]);
    }

    #[test]
    fn test_compute_changes_address_only_in_post() {
        let diff = make_diff(vec![], vec![("0xnew", vec![("0x01", "0xval")])]);
        let changes = compute_changes(&diff);
        assert_eq!(changes.len(), 1);
        assert!(changes.contains_key("0xnew"));
    }

    #[test]
    fn test_compute_changes_address_only_in_pre() {
        let diff = make_diff(vec![("0xold", vec![("0x01", "0xval")])], vec![]);
        let changes = compute_changes(&diff);
        assert_eq!(changes.len(), 1);
        let slots = &changes["0xold"];
        assert_eq!(slots[0].before.as_deref(), Some("0xval"));
        assert_eq!(slots[0].after, None);
    }

    #[test]
    fn test_compute_changes_mixed_operations_single_address() {
        let diff = make_diff(
            vec![(
                "0xaaa",
                vec![("0x01", "0xa"), ("0x02", "0xb"), ("0x03", "0xsame")],
            )],
            vec![(
                "0xaaa",
                vec![("0x01", "0xnew_a"), ("0x03", "0xsame"), ("0x04", "0xd")],
            )],
        );
        let changes = compute_changes(&diff);
        assert_eq!(changes.len(), 1);
        let slots = &changes["0xaaa"];
        assert_eq!(slots.len(), 3);
        assert_eq!(slots[0].slot, "0x01"); // modified
        assert_eq!(slots[0].before.as_deref(), Some("0xa"));
        assert_eq!(slots[0].after.as_deref(), Some("0xnew_a"));
        assert_eq!(slots[1].slot, "0x02"); // deleted
        assert_eq!(slots[1].before.as_deref(), Some("0xb"));
        assert_eq!(slots[1].after, None);
        assert_eq!(slots[2].slot, "0x04"); // new
        assert_eq!(slots[2].before, None);
        assert_eq!(slots[2].after.as_deref(), Some("0xd"));
    }
}
