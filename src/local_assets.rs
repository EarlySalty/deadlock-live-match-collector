use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use deadlock_data::localization::HERO_NAMES_BUNDLE;
use deadlock_data::{vdata, HeroCatalog, LocalizationFile};

const LANGUAGE: &str = "english";
const REFRESH: Duration = Duration::from_secs(30);

#[derive(Debug, PartialEq, Eq)]
pub enum AssetError {
    Archive,
    Roster,
    Localization,
}

fn load(dir: &Path) -> Result<HeroCatalog, AssetError> {
    let roster = vdata::hero_roster(dir).map_err(|_| AssetError::Archive)?;
    let mut ids = HashSet::new();
    let mut classes = HashSet::new();
    if roster.is_empty()
        || roster
            .iter()
            .any(|hero| !ids.insert(hero.id) || !classes.insert(hero.class_name.as_str()))
    {
        return Err(AssetError::Roster);
    }
    let mut catalog = HeroCatalog::new(roster);
    catalog
        .merge_game_dir_lang(dir, LANGUAGE)
        .map_err(|_| AssetError::Localization)?;
    if catalog.localised_count() == 0 {
        return Err(AssetError::Localization);
    }
    Ok(catalog)
}

#[derive(PartialEq, Eq)]
struct FileStamp {
    bytes: u64,
    modified: SystemTime,
}

fn stamp(path: &Path) -> Option<FileStamp> {
    let meta = fs::metadata(path).ok()?;
    if !meta.is_file() {
        return None;
    }
    Some(FileStamp {
        bytes: meta.len(),
        modified: meta.modified().ok()?,
    })
}

fn source_stamp(dir: &Path) -> Option<(FileStamp, FileStamp)> {
    Some((
        stamp(&dir.join(vdata::ARCHIVE))?,
        stamp(&dir.join(LocalizationFile::relative_path(HERO_NAMES_BUNDLE, LANGUAGE)))?,
    ))
}

#[derive(Default)]
pub struct LocalAssets {
    dir: Option<PathBuf>,
    stamp: Option<(FileStamp, FileStamp)>,
    last_attempt: Option<Instant>,
    catalog: HeroCatalog,
}

impl LocalAssets {
    pub fn clear(&mut self) {
        *self = Self::default();
    }

    pub fn refresh(&mut self, dir: Option<PathBuf>, fresh_attach: bool, now: Instant) {
        let Some(dir) = dir else {
            self.clear();
            return;
        };
        let current_stamp = source_stamp(&dir);
        if !fresh_attach
            && self.dir.as_ref() == Some(&dir)
            && self.stamp == current_stamp
            && self
                .last_attempt
                .is_some_and(|last| now.saturating_duration_since(last) < REFRESH)
        {
            return;
        }

        // Invalidate first: an unsuccessful reload must never leave old names usable.
        self.catalog = HeroCatalog::default();
        self.dir = Some(dir.clone());
        self.stamp = current_stamp;
        self.last_attempt = Some(now);
        if self.stamp.is_some() {
            if let Ok(catalog) = load(&dir) {
                if source_stamp(&dir) == self.stamp {
                    self.catalog = catalog;
                }
            }
        }
    }

    pub fn catalog(&self) -> &HeroCatalog {
        &self.catalog
    }
}

#[cfg(test)]
pub fn synthetic_catalog() -> HeroCatalog {
    use deadlock_core::HeroId;
    use deadlock_data::Hero;
    let mut catalog = HeroCatalog::new(vec![Hero {
        id: HeroId(1),
        class_name: "hero_fixture".to_owned(),
        in_development: false,
        disabled: false,
        pre_release: false,
        player_selectable: Some(true),
    }]);
    // The display string is the independent contract vector, not a vendored roster.
    catalog.merge_localization(&LocalizationFile::parse(
        "\"hero_fixture\" \"Infernus\"",
        LANGUAGE,
    ));
    catalog
}

#[cfg(test)]
mod tests {
    use super::*;
    use deadlock_core::HeroId;
    use deadlock_reader::HeroNames;
    use source2::{kv3, resource, vpk};

    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            let dir =
                std::env::temp_dir().join(format!("collector-assets-{}", uuid::Uuid::new_v4()));
            fs::create_dir(&dir).unwrap();
            Self(dir)
        }

        fn names(&self, text: &str) {
            let path = self
                .0
                .join(LocalizationFile::relative_path(HERO_NAMES_BUNDLE, LANGUAGE));
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, text).unwrap();
        }

        fn archive(&self, path: &str, bytes: Vec<u8>, inline: bool) {
            let mut pack = vpk::Vpk::new(2);
            pack.push(if inline {
                vpk::Entry::inline(path, bytes)
            } else {
                vpk::Entry::new(path, bytes)
            });
            pack.write(self.0.join(vdata::ARCHIVE)).unwrap();
        }

        fn roster(&self, text: &str, compression: kv3::Compression, inline: bool) {
            let root = kv3::parse_text(text).unwrap().root;
            let data = kv3::write(
                &root,
                &kv3::WriteOptions {
                    compression,
                    ..Default::default()
                },
            )
            .unwrap();
            let bytes = resource::Resource::new()
                .with_block(resource::Block::new(resource::BlockKind::DATA, data))
                .to_bytes()
                .unwrap();
            self.archive(vdata::HEROES_PATH, bytes, inline);
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    const ROSTER: &str = "{ hero_fixture = { m_HeroID = 1 } hero_other = { m_HeroID = 2 } }";
    const NAMES: &str = "\"hero_fixture\" \"Fixture Name\"";

    #[test]
    fn reads_synthetic_vpk_roster_and_localization_without_fallback() {
        for compression in [
            kv3::Compression::Zstd,
            kv3::Compression::Lz4,
            kv3::Compression::None,
        ] {
            let fixture = Fixture::new();
            fixture.roster(ROSTER, compression, false);
            fixture.names(NAMES);
            let catalog = load(&fixture.0).unwrap();
            assert_eq!(catalog.len(), 2);
            assert!(catalog.is_localised(HeroId(1)));
            assert_eq!(catalog.hero_name(HeroId(1)), Some("Fixture Name"));
            assert!(!catalog.is_localised(HeroId(2)));
            assert!(catalog.hero_name(HeroId(999)).is_none());
        }
    }

    #[test]
    fn missing_corrupt_incompatible_and_wrong_entry_vpk_fail_closed() {
        let fixture = Fixture::new();
        fixture.names(NAMES);
        assert_eq!(load(&fixture.0).unwrap_err(), AssetError::Archive);
        let archive = fixture.0.join(vdata::ARCHIVE);
        fs::write(&archive, b"not a vpk").unwrap();
        assert_eq!(load(&fixture.0).unwrap_err(), AssetError::Archive);
        let mut header = vec![0u8; 28];
        header[..4].copy_from_slice(&0x55aa1234u32.to_le_bytes());
        header[4..8].copy_from_slice(&999u32.to_le_bytes());
        fs::write(&archive, header).unwrap();
        assert_eq!(load(&fixture.0).unwrap_err(), AssetError::Archive);
        fixture.archive("scripts/unrelated.vdata_c", vec![1, 2, 3], true);
        assert_eq!(load(&fixture.0).unwrap_err(), AssetError::Archive);
        fixture.archive(vdata::HEROES_PATH, vec![1, 2, 3], true);
        assert_eq!(load(&fixture.0).unwrap_err(), AssetError::Archive);
        fixture.roster(ROSTER, kv3::Compression::Zstd, false);
        let shard = fixture.0.join("pak01_000.vpk");
        let mut data = fs::read(&shard).unwrap();
        data[0] ^= 1;
        fs::write(&shard, data).unwrap();
        assert_eq!(load(&fixture.0).unwrap_err(), AssetError::Archive);
        fs::remove_file(shard).unwrap();
        assert_eq!(load(&fixture.0).unwrap_err(), AssetError::Archive);
    }

    #[test]
    fn empty_duplicate_or_incompatible_roster_and_missing_names_fail_closed() {
        let fixture = Fixture::new();
        fixture.roster(ROSTER, kv3::Compression::None, true);
        assert_eq!(load(&fixture.0).unwrap_err(), AssetError::Localization);
        fixture.names("malformed / no tokens");
        assert_eq!(load(&fixture.0).unwrap_err(), AssetError::Localization);
        fixture.names("\"hero_unknown\" \"Unknown\"");
        assert_eq!(load(&fixture.0).unwrap_err(), AssetError::Localization);
        fixture.names(NAMES);
        for roster in [
            "{}",
            "{ hero_fixture = { m_HeroID = 0 } }",
            "{ hero_fixture = { incompatible = 1 } }",
            "{ hero_fixture = { m_HeroID = 1 } hero_other = { m_HeroID = 1 } }",
        ] {
            fixture.roster(roster, kv3::Compression::None, true);
            assert_eq!(load(&fixture.0).unwrap_err(), AssetError::Roster);
        }
    }

    #[test]
    fn periodic_reload_detects_corrupt_numbered_archive_even_when_index_unchanged() {
        let fixture = Fixture::new();
        fixture.roster(ROSTER, kv3::Compression::Zstd, false);
        fixture.names(NAMES);
        let now = Instant::now();
        let mut assets = LocalAssets::default();
        assets.refresh(Some(fixture.0.clone()), true, now);
        assert!(!assets.catalog().is_empty());
        let shard = fixture.0.join("pak01_000.vpk");
        let mut bytes = fs::read(&shard).unwrap();
        bytes[0] ^= 1;
        fs::write(shard, bytes).unwrap();
        assets.refresh(Some(fixture.0.clone()), false, now + REFRESH);
        assert!(assets.catalog().is_empty());
    }

    #[test]
    fn cache_reloads_on_attach_change_without_retaining_failed_data() {
        let fixture = Fixture::new();
        fixture.roster(ROSTER, kv3::Compression::None, true);
        fixture.names(NAMES);
        let mut assets = LocalAssets::default();
        let now = Instant::now();
        assets.refresh(Some(fixture.0.clone()), true, now);
        assert_eq!(assets.catalog().hero_name(HeroId(1)), Some("Fixture Name"));
        fixture.names("\"hero_fixture\" \"Renamed Fixture\"");
        assets.refresh(Some(fixture.0.clone()), false, now + Duration::from_secs(1));
        assert_eq!(
            assets.catalog().hero_name(HeroId(1)),
            Some("Renamed Fixture")
        );
        fs::write(fixture.0.join(vdata::ARCHIVE), b"corrupt").unwrap();
        assets.refresh(Some(fixture.0.clone()), false, now + Duration::from_secs(2));
        assert!(assets.catalog().is_empty());
        fixture.roster(ROSTER, kv3::Compression::None, true);
        assets.refresh(Some(fixture.0.clone()), true, now + Duration::from_secs(3));
        assert_eq!(
            assets.catalog().hero_name(HeroId(1)),
            Some("Renamed Fixture")
        );
        assets.refresh(None, false, now + Duration::from_secs(4));
        assert!(assets.catalog().is_empty());
        assets.refresh(Some(fixture.0.clone()), true, now + Duration::from_secs(5));
        assert!(!assets.catalog().is_empty());
        assets.clear();
        assert!(assets.catalog().is_empty());
        let other = Fixture::new();
        assets.refresh(Some(other.0.clone()), true, now + REFRESH);
        assert!(assets.catalog().is_empty());
    }
}
