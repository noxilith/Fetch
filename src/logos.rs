//! Logos built into the binary, used when the distro is detected (or named with `--logo`).

/// The SingularitySYS logo: a ringed black hole, traced from the project's own SVG.
/// Characters are picked by relief height (. lowest .. M tallest) so the 3D render
/// rounds off at the edges.
const SINGULARITY: [&str; 16] = [
    "                   +ohmNNMMNNdhy+",
    "                ydMMMMMMMMMMMMMMMNh+",
    "              oNMMMMMMmhooohdmMMMMMMNy:       :syohdmmmh",
    "            :NMMMMNh+          sdMMMMMMNddmNMMMNdhdmMmy",
    "           +MMMMMo               :dMMMMMMMMmhyyomMNo:",
    "           NMMMMs                  hMMMNhysomMMNh+",
    "          oMMMMo                :ydMdy+sdMMMMh+",
    "          mMMMM:             shNmo+ :dMMMMNy",
    "         sNMMMM:         :ydNdy:    yMMMMh",
    "       :dMMMMMMo     :yhNmo+        mMMMM+",
    "     :dMMMMMMMMMdhdmMNds           oMMMMm",
    "   :hMMMMMMMMMMMMMmo+            :dMMMMN",
    "  hMMMMMMMMMMNhy+              sdMMMMMd",
    "sMMMMMMMmhy+     :odNNmhooohdmMMMMMMNs",
    "ohhoy+:         :dMMMMMMMMMMMMMMMMh+",
    "                   +yhmNNMMNNdhy+",
];

/// The built-in logo for a distro name or os-release ID, if there is one.
pub fn builtin(distro: &str) -> Option<&'static [&'static str]> {
    match distro.to_ascii_lowercase().as_str() {
        "singularity" | "singularitysys" => Some(&SINGULARITY),
        _ => None,
    }
}

/// The built-in art flattened to the classic `@%#` look for static (`--minimal`)
/// output: the density characters only exist to give the 3D render its relief.
pub fn flatten(row: &str) -> String {
    row.chars()
        .map(|c| match crate::logo::char_weight(c) {
            _ if c == ' ' => ' ',
            w if w >= 0.60 => '@',
            w if w >= 0.30 => '%',
            _ => '#',
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::logo::{MAX_LOGO_COLS, MAX_LOGO_ROWS};

    #[test]
    fn singularity_matches_by_id_or_name() {
        for name in ["singularity", "SingularitySYS", "SINGULARITY"] {
            assert!(builtin(name).is_some(), "{name}");
        }
        assert!(builtin("arch").is_none());
        assert!(builtin("").is_none());
    }

    #[test]
    fn flatten_keeps_the_shape() {
        assert_eq!(flatten(" .oM"), " #%@");
        let art = builtin("singularity").unwrap();
        for row in art {
            let flat = flatten(row);
            assert_eq!(flat.chars().count(), row.chars().count());
            assert!(flat.chars().all(|c| " @%#".contains(c)));
        }
    }

    #[test]
    fn builtin_logos_fit_the_limits() {
        let art = builtin("singularity").unwrap();
        assert!(art.len() <= MAX_LOGO_ROWS);
        assert!(art.iter().all(|r| r.chars().count() <= MAX_LOGO_COLS));
        assert!(art.iter().any(|r| r.contains('M')));
        // wider than a row of the 60-column canvas would clip it while spinning
        assert!(art.iter().all(|r| r.chars().count() <= 56));
    }
}
