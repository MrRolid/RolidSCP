//! Master password policy. All rules live in [`check`] – one pure function, easy to
//! change and covered by unit tests.

use rolid_text::{nc, txt, txtn, Text};

/// Minimum length in characters (Unicode scalar values, not bytes).
pub const MIN_CHARS: usize = 10;
/// Minimum number of uppercase letters (Unicode, not only A–Z).
pub const MIN_UPPER: usize = 2;
/// Minimum zxcvbn score (0–4).
pub const MIN_SCORE: u8 = 3;
/// Names that must not appear in the password.
const APP_NAMES: &[&str] = &["rolidscp", "rolid"];
/// Keyboard rows / alphabets used to detect plain sequences.
const SEQUENCES: &[&str] = &[
    "abcdefghijklmnopqrstuvwxyz",
    "0123456789",
    "qwertyuiop",
    "qwertzuiop",
    "asdfghjkl",
    "yxcvbnm",
    "zxcvbnm",
    "1234567890",
];

/// Result of the check: score for the strength meter and reasons for rejection.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Strength {
    /// zxcvbn score 0–4 (0 for an empty password).
    pub score: u8,
    /// Why the password is not acceptable; empty = acceptable.
    pub reasons: Vec<Text>,
}

impl Strength {
    pub fn ok(&self) -> bool {
        self.reasons.is_empty()
    }

    /// Short label for the meter (English, marked with context "password";
    /// the UI translates it).
    pub fn label(&self) -> &'static str {
        match (self.ok(), self.score) {
            (_, 0) => nc!("password", "very weak"),
            (_, 1) => nc!("password", "weak"),
            (_, 2) => nc!("password", "fair"),
            (false, _) => nc!("password", "not acceptable"),
            (true, 3) => nc!("password", "strong"),
            (true, _) => nc!("password", "very strong"),
        }
    }
}

/// Check a master password. `user_inputs` are words that must not be used
/// (user name etc.). Never logs or stores the password.
pub fn check(password: &str, user_inputs: &[&str]) -> Strength {
    let mut reasons = Vec::new();
    let chars: Vec<char> = password.chars().collect();
    let n = chars.len();
    if n < MIN_CHARS {
        let missing = MIN_CHARS - n;
        reasons.push(txtn!(
            "too short – {n} more character needed (the minimum is {min})",
            "too short – {n} more characters needed (the minimum is {min})",
            missing,
            min = MIN_CHARS
        ));
    }
    let upper = chars.iter().filter(|c| c.is_uppercase()).count();
    match upper {
        0 => reasons.push(txt!("no uppercase letters (at least 2 needed)")),
        1 => reasons.push(txt!("a second uppercase letter is missing")),
        _ => {}
    }
    if !chars.iter().any(|c| !c.is_alphabetic()) {
        reasons.push(txt!("a digit or another character than a letter is missing (e.g. 7, !, space)"));
    }
    if n == 0 {
        return Strength { score: 0, reasons };
    }

    let lower = password.to_lowercase();
    if APP_NAMES.iter().any(|a| lower.contains(a)) {
        reasons.push(txt!("contains the application name"));
    }
    if user_inputs
        .iter()
        .map(|u| u.trim().to_lowercase())
        .any(|u| u.chars().count() >= 3 && lower.contains(&u))
    {
        reasons.push(txt!("contains the user name"));
    }
    if longest_repeat(&chars) * 2 >= n.max(8) {
        reasons.push(txt!("almost all of it is one repeated character"));
    }
    if longest_sequence(&lower) * 2 >= n.max(10) {
        reasons.push(txt!("it is mostly a plain sequence of characters (abcdef, 123456, qwerty)"));
    }

    let mut inputs: Vec<&str> = APP_NAMES.to_vec();
    inputs.extend(user_inputs.iter().copied().filter(|u| !u.trim().is_empty()));
    let entropy = zxcvbn::zxcvbn(password, &inputs);
    let score = u8::from(entropy.score());
    if score < MIN_SCORE && reasons.is_empty() {
        use zxcvbn::feedback::Warning::*;
        let why = match entropy.feedback().and_then(|f| f.warning()) {
            Some(ThisIsATop10Password | ThisIsATop100Password | ThisIsACommonPassword) => {
                txt!("too common a password – add more unrelated words or characters")
            }
            Some(ThisIsSimilarToACommonlyUsedPassword) => {
                txt!("too similar to a common password – add more unrelated words or characters")
            }
            Some(StraightRowsOfKeysAreEasyToGuess | ShortKeyboardPatternsAreEasyToGuess) => {
                txt!("contains an easy-to-guess row of keys – add more unrelated words or characters")
            }
            Some(RepeatsLikeAaaAreEasyToGuess | RepeatsLikeAbcAbcAreOnlySlightlyHarderToGuess) => {
                txt!("contains an easy-to-guess repetition – add more unrelated words or characters")
            }
            Some(SequencesLikeAbcAreEasyToGuess) => {
                txt!("contains an easy-to-guess sequence – add more unrelated words or characters")
            }
            Some(RecentYearsAreEasyToGuess | DatesAreOftenEasyToGuess) => {
                txt!("contains a year or a date, which is easy to guess – add more unrelated words or characters")
            }
            Some(AWordByItselfIsEasyToGuess) => {
                txt!("it is just a common word with a small change – add more unrelated words or characters")
            }
            Some(
                NamesAndSurnamesByThemselvesAreEasyToGuess | CommonNamesAndSurnamesAreEasyToGuess,
            ) => txt!("contains a common first name or surname – add more unrelated words or characters"),
            _ => txt!("easy to guess – add more unrelated words or characters"),
        };
        reasons.push(why);
    }
    Strength { score, reasons }
}

/// Longest run of one repeated character.
fn longest_repeat(chars: &[char]) -> usize {
    let mut best = 0;
    let mut run = 0;
    let mut prev = None;
    for &c in chars {
        run = if Some(c) == prev { run + 1 } else { 1 };
        prev = Some(c);
        best = best.max(run);
    }
    best
}

/// Longest run of characters that follow a known sequence (forwards or backwards).
fn longest_sequence(lower: &str) -> usize {
    let chars: Vec<char> = lower.chars().collect();
    let mut best = 0;
    for seq in SEQUENCES {
        let s: Vec<char> = seq.chars().collect();
        for dir in [1isize, -1] {
            let mut run = 0;
            for w in chars.windows(2) {
                let next = s.iter().position(|&c| c == w[0]).and_then(|i| {
                    let j = i as isize + dir;
                    (j >= 0).then(|| s.get(j as usize)).flatten()
                });
                if next == Some(&w[1]) {
                    run = if run == 0 { 2 } else { run + 1 };
                } else {
                    run = 0;
                }
                best = best.max(run);
            }
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;

    const GOOD: &str = "Modrý Kůň skáče 7x";

    fn reasons(pw: &str) -> Vec<String> {
        check(pw, &["user"]).reasons.iter().map(|r| r.to_string()).collect()
    }

    fn reasons_of(s: &Strength) -> Vec<String> {
        s.reasons.iter().map(|r| r.to_string()).collect()
    }

    #[test]
    fn good_password_passes() {
        let s = check(GOOD, &["user"]);
        assert!(s.ok(), "{:?}", s.reasons);
        assert!(s.score >= MIN_SCORE);
    }

    #[test]
    fn length_boundary_counts_characters_not_bytes() {
        // 9 characters (but more than 10 bytes thanks to diacritics) – too short.
        let nine = "ŽÁřč7ěšťň";
        assert_eq!(nine.chars().count(), 9);
        assert!(nine.len() > 10);
        assert!(reasons(nine).iter().any(|r| r.contains("1 more character needed")), "{:?}", reasons(nine));
        // 10 characters: the length rule is satisfied.
        let ten = "ŽÁřč7ěšťňx";
        assert!(!reasons(ten).iter().any(|r| r.contains("too short")), "{:?}", reasons(ten));
    }

    #[test]
    fn uppercase_boundary_and_unicode() {
        assert!(reasons("River without bridge 42").iter().any(|r| r == "a second uppercase letter is missing"));
        assert!(reasons("river without bridge 42").iter().any(|r| r.starts_with("no uppercase letters")));
        // Czech / Cyrillic / Greek capitals count as uppercase.
        let s = check("Žluťoučký Řeřicha 42", &[]);
        assert!(!reasons_of(&s).iter().any(|r| r.contains("uppercase")), "{:?}", s.reasons);
        let s = check("Ёжик Ωmega туман 9", &[]);
        assert!(!reasons_of(&s).iter().any(|r| r.contains("uppercase")), "{:?}", s.reasons);
    }

    #[test]
    fn needs_digit_or_non_letter() {
        let r = reasons("ŽluťoučkýKůňBěžíRychle");
        assert!(r.iter().any(|r| r.starts_with("a digit")), "{r:?}");
        // A space alone is enough.
        let r = reasons("Žluťoučký Kůň běží rychle");
        assert!(!r.iter().any(|r| r.starts_with("a digit")), "{r:?}");
    }

    #[test]
    fn weak_passwords_that_pass_the_basic_rules_are_rejected() {
        for pw in [
            "PassWord123",  // dictionary
            "PAssword2024", // dictionary + year
            "QWertyuiop1",  // keyboard row
            "ABcdefghij1",  // alphabet sequence
            "AAAAAAAAAA1",  // one repeated letter
            "XY1111111111", // one repeated digit
            "Xy1234567890", // digit sequence
            "RolidSCP2024!", // app name
            "User!User!X1", // user name
        ] {
            let s = check(pw, &["user"]);
            assert!(!s.ok(), "{pw} should be rejected (score {})", s.score);
        }
        assert!(reasons("PassWord123").iter().any(|r| r.contains("common")), "{:?}", reasons("PassWord123"));
        assert!(reasons("RolidSCP2024!").iter().any(|r| r == "contains the application name"));
        assert!(reasons("User!User!X1").iter().any(|r| r == "contains the user name"));
        assert!(reasons("AAAAAAAAAA1").iter().any(|r| r.contains("repeated")));
        assert!(reasons("ABcdefghij1").iter().any(|r| r.contains("sequence")));
    }

    #[test]
    fn empty_password() {
        let s = check("", &[]);
        assert_eq!(s.score, 0);
        assert!(!s.ok());
    }
}
