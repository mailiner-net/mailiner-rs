//! Local pin overlay: keep selected folder rows at the top of the current list.

/// Stable partition: pinned UIDs first (in `pinned_uids` order), then the rest.
///
/// UIDs missing from `items` are ignored. Returns whether the slice changed.
pub fn sort_pinned_first<T: Clone>(
    items: &mut [T],
    pinned_uids: &[String],
    uid_of: impl Fn(&T) -> &str,
) -> bool {
    if items.len() < 2 || pinned_uids.is_empty() {
        return false;
    }

    let mut used = vec![false; items.len()];
    let mut next = Vec::with_capacity(items.len());
    for uid in pinned_uids {
        if let Some(idx) = items
            .iter()
            .enumerate()
            .find(|(i, item)| !used[*i] && uid_of(item) == uid.as_str())
            .map(|(i, _)| i)
        {
            used[idx] = true;
            next.push(items[idx].clone());
        }
    }
    if next.is_empty() {
        return false;
    }
    for (i, keep) in used.iter().enumerate() {
        if !keep {
            next.push(items[i].clone());
        }
    }
    if next
        .iter()
        .zip(items.iter())
        .all(|(a, b)| uid_of(a) == uid_of(b))
    {
        return false;
    }
    items.clone_from_slice(&next);
    true
}

/// Whether every listed UID is already pinned (empty → treat as off).
pub fn all_pinned(uids: impl IntoIterator<Item = impl AsRef<str>>, pinned: &[String]) -> bool {
    let mut any = false;
    let mut all_on = true;
    for uid in uids {
        any = true;
        all_on &= pinned.iter().any(|p| p == uid.as_ref());
    }
    any && all_on
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_or_single_is_noop() {
        let mut none: [&str; 0] = [];
        assert!(!sort_pinned_first(&mut none, &["1".into()], |s| s));
        let mut one = ["a"];
        assert!(!sort_pinned_first(&mut one, &["a".into()], |s| s));
    }

    #[test]
    fn missing_pins_are_ignored() {
        let mut items = ["a", "b", "c"];
        assert!(!sort_pinned_first(&mut items, &["z".into()], |s| *s));
        assert_eq!(items, ["a", "b", "c"]);
    }

    #[test]
    fn pinned_rows_move_to_front_in_pin_order() {
        let mut items = ["a", "b", "c", "d"];
        assert!(sort_pinned_first(
            &mut items,
            &["c".into(), "a".into()],
            |s| *s
        ));
        assert_eq!(items, ["c", "a", "b", "d"]);
        assert!(!sort_pinned_first(
            &mut items,
            &["c".into(), "a".into()],
            |s| *s
        ));
    }

    #[test]
    fn already_first_is_noop() {
        let mut items = ["b", "a", "c"];
        assert!(!sort_pinned_first(&mut items, &["b".into()], |s| *s));
    }

    #[test]
    fn all_pinned_requires_every_uid() {
        let pins = vec!["1".into(), "2".into()];
        assert!(all_pinned(["1", "2"], &pins));
        assert!(!all_pinned(["1", "3"], &pins));
        assert!(!all_pinned(std::iter::empty::<&str>(), &pins));
    }
}
