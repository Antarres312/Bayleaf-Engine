//! The editor's translations: English source text looked up in the craft-i18n catalogs
//! (`craft_i18n`), embedded at build time when `CRAFT_I18N_DIR` names a craft-i18n checkout
//! (`build.rs`). A missing translation shows the English. The engine, the CLI and agents stay in
//! English; only what the editor draws is translated.
//!
//! [`tl`] translates a label, [`tid`] a command's label by its id, [`tc`] a label with a context
//! (`menu`), and [`tf`] a message with `{name}` placeholders. The language is per thread (the
//! editor draws on one), so tests can switch it without touching each other.

use std::cell::Cell;
use std::sync::LazyLock;

use craft_i18n::{Direction, Embedded, I18n, Lang};

/// The embedded catalogs: (locale, shared catalog, the editor's catalog); empty without `CRAFT_I18N_DIR`.
static CRAFT_I18N: &Embedded = include!(concat!(env!("OUT_DIR"), "/craft_i18n.rs"));

/// The lookups. The editor offers every embedded language written left to right; right-to-left
/// languages wait for a mirrored layout.
static I18N: LazyLock<I18n> = LazyLock::new(|| {
    let offered: Vec<&str> = CRAFT_I18N.iter().map(|(code, _, _)| *code).filter(|code| craft_i18n::direction(code) == Direction::Ltr).collect();
    let (i18n, _problems) = I18n::new(CRAFT_I18N, &offered);
    i18n
});

thread_local! {
    static CURRENT: Cell<Lang> = const { Cell::new(Lang::EN) };
}

/// The setting that follows the system's language.
pub const AUTOMATIC: &str = "auto";

/// `source` in the editor's language.
pub fn tl(source: &str) -> &str {
    I18N.t(CURRENT.get(), source)
}

/// `source` with a context such as `menu`, else as [`tl`].
pub fn tc<'a>(context: &str, source: &'a str) -> &'a str {
    I18N.tc(CURRENT.get(), context, source)
}

/// A command's label, by its id (`@id` rows), else its English label translated.
pub fn tid<'a>(id: &str, label: &'a str) -> &'a str {
    I18N.tid(CURRENT.get(), id, label)
}

/// A message with `{name}` placeholders, translated, then filled in.
pub fn tf(template: &str, args: &[(&str, &str)]) -> String {
    craft_i18n::fmt(tl(template), args)
}

/// A count's wording in the editor's language: `one` and `other` are the English forms, with
/// `{n}` for the number.
pub fn tn(n: u64, one: &str, other: &str) -> String {
    I18N.trn(CURRENT.get(), n, one, other)
}

/// Switches the editor to a language setting: [`AUTOMATIC`] for the system's language, or a code
/// (`pl`, `pt-br`; `pseudo` in debug builds). An unknown code is English. Returns the code in use.
pub fn set_language(setting: &str) -> &'static str {
    let lang = if setting == AUTOMATIC { I18N.system_lang() } else { I18N.from_code(setting).unwrap_or(Lang::EN) };
    CURRENT.set(lang);
    I18N.code(lang)
}

/// The code of the language in use.
pub fn language() -> &'static str {
    I18N.code(CURRENT.get())
}

/// The languages to choose from: (code, name in itself), English first. Debug builds add the
/// pseudo locale, which marks every translated text so untranslated text stands out.
pub fn languages() -> Vec<(&'static str, &'static str)> {
    let mut out: Vec<(&str, &str)> = I18N.languages().into_iter().map(|lang| (I18N.code(lang), I18N.name(lang))).collect();
    if cfg!(debug_assertions) {
        out.push(("pseudo", "Pseudo (marks translated text)"));
    }
    out
}

/// The name of the system's language, for the Automatic choice.
pub fn system_language_name() -> &'static str {
    I18N.name(I18N.system_lang())
}

/// The codes the font stack prepares faces for: every offered language.
pub fn language_codes() -> Vec<&'static str> {
    I18N.languages().into_iter().map(|lang| I18N.code(lang)).collect()
}

#[cfg(test)]
pub(crate) fn looked_up() -> std::collections::BTreeSet<String> {
    I18N.looked_up()
}
