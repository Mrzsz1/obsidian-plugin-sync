use crate::models::PluginDiffStatus;
use semver::Version;

pub fn compare_versions(source: Option<&str>, target: Option<&str>) -> PluginDiffStatus {
    match (source, target) {
        (Some(source), Some(target)) if source == target => PluginDiffStatus::SameVersion,
        (Some(source), Some(target)) => {
            let parsed_source = parse_version(source);
            let parsed_target = parse_version(target);
            match (parsed_source, parsed_target) {
                (Some(source), Some(target)) if source > target => PluginDiffStatus::SourceNewer,
                (Some(source), Some(target)) if source < target => PluginDiffStatus::SourceOlder,
                (Some(_), Some(_)) => PluginDiffStatus::SameVersion,
                _ => PluginDiffStatus::VersionDifferentUnknown,
            }
        }
        _ => PluginDiffStatus::VersionDifferentUnknown,
    }
}

fn parse_version(version: &str) -> Option<Version> {
    let normalized = version.trim().trim_start_matches('v');
    Version::parse(normalized).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compares_semver_versions() {
        assert_eq!(
            compare_versions(Some("1.2.0"), Some("1.1.0")),
            PluginDiffStatus::SourceNewer
        );
        assert_eq!(
            compare_versions(Some("1.0.0"), Some("1.1.0")),
            PluginDiffStatus::SourceOlder
        );
        assert_eq!(
            compare_versions(Some("v1.0.0"), Some("1.0.0")),
            PluginDiffStatus::SameVersion
        );
    }

    #[test]
    fn reports_unknown_for_non_semver() {
        assert_eq!(
            compare_versions(Some("2026.07-beta"), Some("2026.06")),
            PluginDiffStatus::VersionDifferentUnknown
        );
    }
}
