//! Version ordering for installation; no downgrade or replacement of unknown files.
pub(crate) type Version = (u32, u32);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Existing {
    Missing,
    Unknown,
    Collector(Version),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Action {
    Install,
    Replace,
    LaunchInstalled,
    Refuse,
}

pub(crate) fn action(incoming: Version, existing: Existing) -> Action {
    match existing {
        Existing::Missing => Action::Install,
        Existing::Unknown => Action::Refuse,
        Existing::Collector(installed) if incoming > installed => Action::Replace,
        Existing::Collector(_) => Action::LaunchInstalled,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_newer_collector_replaces_an_installed_collector() {
        let current = (2, 4 << 16);
        assert_eq!(action(current, Existing::Missing), Action::Install);
        assert_eq!(action(current, Existing::Unknown), Action::Refuse);
        assert_eq!(
            action(current, Existing::Collector((2, 3 << 16))),
            Action::Replace
        );
        assert_eq!(
            action(current, Existing::Collector(current)),
            Action::LaunchInstalled
        );
        assert_eq!(
            action(current, Existing::Collector((2, 5 << 16))),
            Action::LaunchInstalled
        );
        assert_eq!(
            action(current, Existing::Collector((3, 0))),
            Action::LaunchInstalled
        );
    }

    #[test]
    fn version_order_is_numeric_and_handles_major_minor_boundaries() {
        assert_eq!(
            action((2, 10 << 16), Existing::Collector((2, 9 << 16))),
            Action::Replace
        );
        assert_eq!(
            action((3, 0), Existing::Collector((2, 99 << 16))),
            Action::Replace
        );
        assert_eq!(
            action((1 << 16, 0), Existing::Collector((99, 0))),
            Action::Replace
        );
    }
}
