//! Public, read-only release discovery. Never authenticates or downloads binaries.
use serde::Deserialize;
use std::{cmp::Ordering, io::Read, time::Duration};

pub(crate) const INTERVAL: Duration = Duration::from_secs(6 * 60 * 60);
const API: &str = "https://api.github.com/repos/EarlySalty/deadlock-live-match-collector/releases";
const PAGE_LIMIT: usize = 20;
const BODY_LIMIT: u64 = 4 * 1024 * 1024;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Version {
    core: [u32; 3],
    pre: Vec<String>,
}
impl Version {
    pub(crate) fn parse(tag: &str) -> Option<Self> {
        let tag = tag.strip_prefix('v').unwrap_or(tag);
        if tag.len() > 100 {
            return None;
        }
        let (core, pre) = tag.split_once('-').unwrap_or((tag, ""));
        let parts: Vec<_> = core.split('.').collect();
        if parts.len() != 3 {
            return None;
        }
        let numeric = |s: &str| {
            !s.is_empty()
                && s.bytes().all(|b| b.is_ascii_digit())
                && (s == "0" || !s.starts_with('0'))
        };
        if !parts.iter().all(|s| numeric(s)) {
            return None;
        }
        let core = [
            parts[0].parse().ok()?,
            parts[1].parse().ok()?,
            parts[2].parse().ok()?,
        ];
        let mut ids = Vec::new();
        if tag.contains('-') {
            for id in pre.split('.') {
                if id.is_empty()
                    || !id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
                    || (id.bytes().all(|b| b.is_ascii_digit()) && !numeric(id))
                {
                    return None;
                }
                ids.push(id.to_owned());
            }
        }
        Some(Self { core, pre: ids })
    }
}
impl Ord for Version {
    fn cmp(&self, other: &Self) -> Ordering {
        self.core.cmp(&other.core).then_with(|| {
            match (self.pre.is_empty(), other.pre.is_empty()) {
                (true, false) => return Ordering::Greater,
                (false, true) => return Ordering::Less,
                _ => {}
            }
            for (a, b) in self.pre.iter().zip(&other.pre) {
                let an = a.bytes().all(|c| c.is_ascii_digit());
                let bn = b.bytes().all(|c| c.is_ascii_digit());
                let order = match (an, bn) {
                    (true, true) => a.len().cmp(&b.len()).then_with(|| a.cmp(b)),
                    (true, false) => Ordering::Less,
                    (false, true) => Ordering::Greater,
                    _ => a.cmp(b),
                };
                if order != Ordering::Equal {
                    return order;
                }
            }
            self.pre.len().cmp(&other.pre.len())
        })
    }
}
impl PartialOrd for Version {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
#[derive(Deserialize)]
struct Release {
    tag_name: String,
    draft: bool,
    prerelease: bool,
    published_at: Option<String>,
    body: Option<String>,
    html_url: String,
    #[serde(default)]
    assets: Vec<Asset>,
}
#[derive(Deserialize)]
struct Asset {
    name: String,
    state: String,
}
#[derive(Clone, Debug)]
pub(crate) struct Update {
    pub(crate) version: Version,
    pub(crate) tag: String,
    pub(crate) notes: String,
    pub(crate) url: String,
}
fn select(releases: Vec<Release>, installed: &Version, test_channel: bool) -> Option<Update> {
    releases
        .into_iter()
        .filter_map(|r| {
            let version = Version::parse(&r.tag_name)?;
            let url = format!(
                "https://github.com/EarlySalty/deadlock-live-match-collector/releases/tag/{}",
                r.tag_name
            );
            if r.draft
                || r.published_at.as_deref().is_none_or(str::is_empty)
                || (!test_channel && (r.prerelease || !version.pre.is_empty()))
                || &version <= installed
                || r.html_url != url
                || !r
                    .assets
                    .iter()
                    .any(|a| a.name == "live-match-collector.exe" && a.state == "uploaded")
            {
                return None;
            }
            Some(Update {
                version,
                tag: r.tag_name,
                notes: r.body.unwrap_or_default(),
                url,
            })
        })
        .max_by(|a, b| a.version.cmp(&b.version))
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CheckError {
    Network,
    RateLimited,
    Unavailable,
    Invalid,
}
fn status(code: u16) -> Result<(), CheckError> {
    match code {
        200 => Ok(()),
        403 | 429 => Err(CheckError::RateLimited),
        404 => Err(CheckError::Unavailable),
        _ => Err(CheckError::Network),
    }
}
pub(crate) fn check(test_channel: bool) -> Result<Option<Update>, CheckError> {
    let installed = Version::parse(env!("CARGO_PKG_VERSION")).ok_or(CheckError::Invalid)?;
    let client = crate::client_builder()
        .build()
        .map_err(|_| CheckError::Network)?;
    let mut best: Option<Update> = None;
    for page in 1..=PAGE_LIMIT {
        let response = client
            .get(API)
            .query(&[("per_page", 100), ("page", page)])
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28")
            .send()
            .map_err(|_| CheckError::Network)?;
        status(response.status().as_u16())?;
        let mut bytes = Vec::new();
        response
            .take(BODY_LIMIT + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| CheckError::Network)?;
        if bytes.len() as u64 > BODY_LIMIT {
            return Err(CheckError::Invalid);
        }
        let releases: Vec<Release> =
            serde_json::from_slice(&bytes).map_err(|_| CheckError::Invalid)?;
        let last = releases.len() < 100;
        if let Some(candidate) = select(releases, &installed, test_channel) {
            if best
                .as_ref()
                .is_none_or(|old| candidate.version > old.version)
            {
                best = Some(candidate);
            }
        }
        if last {
            return Ok(best);
        }
    }
    Err(CheckError::Invalid)
}
#[derive(Default)]
pub(crate) struct Notice {
    dismissed: Option<Version>,
    pub(crate) running: bool,
    pub(crate) manual: bool,
}
impl Notice {
    pub(crate) fn new(dismissed: Option<Version>) -> Self {
        Self {
            dismissed,
            ..Self::default()
        }
    }
    pub(crate) fn begin(&mut self, manual: bool) -> bool {
        self.manual |= manual;
        if self.running {
            return false;
        }
        self.running = true;
        true
    }
    pub(crate) fn finish(&mut self) -> bool {
        self.running = false;
        std::mem::take(&mut self.manual)
    }
    pub(crate) fn should_show(&self, update: &Update, manual: bool) -> bool {
        manual || self.dismissed.as_ref().is_none_or(|v| &update.version > v)
    }
    pub(crate) fn dismiss(&mut self, update: &Update) {
        if self.dismissed.as_ref().is_none_or(|v| &update.version > v) {
            self.dismissed = Some(update.version.clone());
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn v(s: &str) -> Version {
        Version::parse(s).unwrap()
    }
    fn release(tag: &str, draft: bool, prerelease: bool) -> Release {
        Release {
            tag_name: tag.into(),
            draft,
            prerelease,
            published_at: Some("2026-01-01".into()),
            body: Some("Original notes".into()),
            assets: vec![Asset {
                name: "live-match-collector.exe".into(),
                state: "uploaded".into(),
            }],
            html_url: format!(
                "https://github.com/EarlySalty/deadlock-live-match-collector/releases/tag/{tag}"
            ),
        }
    }
    #[test]
    fn numeric_and_prerelease_order() {
        assert!(v("1.0.0") > v("0.99.99"));
        assert!(v("v0.10.0") > v("0.9.99"));
        assert!(v("0.2.10") > v("0.2.9"));
        assert!(v("1.0.0") > v("1.0.0-rc.10"));
        assert!(v("1.0.0-rc.10") > v("1.0.0-rc.2"));
        for invalid in [
            "internal-test-abc",
            "1.2",
            "1.02.3",
            "1.2.3-",
            "1.2.3-01",
            "1.2.3/evil",
        ] {
            assert!(Version::parse(invalid).is_none());
        }
    }
    #[test]
    fn only_published_appropriate_releases() {
        let list = || {
            vec![
                release("v1.0.0", false, false),
                release("v2.0.0-rc.1", false, true),
                release("v9.0.0", true, false),
                release("v0.1.0", false, false),
            ]
        };
        assert_eq!(select(list(), &v("0.2.9"), false).unwrap().tag, "v1.0.0");
        assert_eq!(
            select(list(), &v("0.2.9"), true).unwrap().tag,
            "v2.0.0-rc.1"
        );
        let mut invalid = release("v9.0.0", false, false);
        invalid.html_url = "https://example.com".into();
        let mut without_binary = release("v10.0.0", false, false);
        without_binary.assets.clear();
        assert!(select(vec![without_binary], &v("0.2.9"), false).is_none());
        let mut unpublished = release("v8.0.0", false, false);
        unpublished.published_at = None;
        assert!(select(vec![invalid, unpublished], &v("0.2.9"), false).is_none());
        assert!(select(list(), &v("2.0.0"), true).is_none());
    }
    #[test]
    fn notes_and_notice_deduplication() {
        let u = select(vec![release("v1.0.0", false, false)], &v("0.2.9"), false).unwrap();
        assert_eq!(u.notes, "Original notes");
        assert_eq!(
            u.url,
            "https://github.com/EarlySalty/deadlock-live-match-collector/releases/tag/v1.0.0"
        );
        let mut n = Notice::new(None);
        assert!(n.begin(false));
        assert!(!n.begin(true));
        assert!(n.finish());
        assert!(n.should_show(&u, false));
        n.dismiss(&u);
        assert!(!n.should_show(&u, false));
        assert!(n.should_show(&u, true));
        assert!(!Notice::new(Some(v("1.0.0"))).should_show(&u, false));
        let next = select(vec![release("v1.0.1", false, false)], &v("0.2.9"), false).unwrap();
        assert!(n.should_show(&next, false));
    }
    #[test]
    fn failures_are_not_updates() {
        assert_eq!(status(403), Err(CheckError::RateLimited));
        assert_eq!(status(429), Err(CheckError::RateLimited));
        assert_eq!(status(404), Err(CheckError::Unavailable));
        assert_eq!(status(500), Err(CheckError::Network));
        assert!(serde_json::from_str::<Vec<Release>>("{}").is_err());
        assert_eq!(INTERVAL.as_secs(), 21600);
        // Verify transport constants without touching the network.
        assert!(API.starts_with("https://api.github.com/"));
        let _ = check as fn(bool) -> Result<Option<Update>, CheckError>;
    }
}
