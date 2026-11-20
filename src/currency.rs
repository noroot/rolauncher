// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2025-2026 dmth

//! Currency conversion: query parsing (`100 USD to EUR`), exchange-rate cache,
//! and background fetching from open.er-api.com (free, no API key, daily updates).

use std::collections::HashMap;
use std::ffi::c_void;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::calc;

pub const RATES_URL: &str = "https://open.er-api.com/v6/latest/USD";
pub const ATTRIBUTION_URL: &str = "https://www.exchangerate-api.com";
/// Refetch when the cached rates are older than this.
const MAX_AGE_SECS: u64 = 6 * 3600;
/// After a failed fetch, wait this long before trying again.
const RETRY_SECS: u64 = 60;

// ---------------------------------------------------------------------------
// Query parsing
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub struct Query {
    pub amount: f64,
    pub from: String,
    pub to: String,
}

const SYMBOLS: &[(char, &str)] = &[
    ('$', "USD"),
    ('€', "EUR"),
    ('£', "GBP"),
    ('¥', "JPY"),
    ('₽', "RUB"),
    ('₴', "UAH"),
    ('₹', "INR"),
    ('₩', "KRW"),
    ('₺', "TRY"),
    ('₪', "ILS"),
    ('₿', "BTC"),
];

const CONNECTORS: &[&str] = &[" to ", " in ", " into ", " as ", "->", "=", "→"];

/// Parses `<amount> <currency> to <currency>`. The amount may be an arithmetic
/// expression and the source currency may be a prefix/suffix symbol (`$100`, `100€`).
pub fn parse_query(input: &str) -> Option<Query> {
    let s = input.trim().to_lowercase();
    let (lhs, rhs) = CONNECTORS.iter().find_map(|c| {
        let i = s.rfind(c)?;
        Some((&s[..i], &s[i + c.len()..]))
    })?;
    let to = currency_code(rhs.trim())?;
    let (amount, from) = split_amount(lhs.trim())?;
    Some(Query { amount, from, to })
}

fn split_amount(s: &str) -> Option<(f64, String)> {
    let first = s.chars().next()?;
    let last = s.chars().next_back()?;
    let (amount, code) = if let Some(code) = symbol_code(first) {
        (&s[first.len_utf8()..], code.to_string())
    } else if let Some(code) = symbol_code(last) {
        (&s[..s.len() - last.len_utf8()], code.to_string())
    } else {
        let split = s.trim_end_matches(|c: char| c.is_ascii_alphabetic()).len();
        (&s[..split], currency_code(&s[split..])?)
    };
    // Commas/underscores are thousands separators: "1,000" == "1000".
    let amount: String = amount.chars().filter(|c| !matches!(c, ',' | '_')).collect();
    let amount = calc::evaluate(amount.trim())?;
    Some((amount, code))
}

fn symbol_code(c: char) -> Option<&'static str> {
    SYMBOLS.iter().find(|(s, _)| *s == c).map(|(_, code)| *code)
}

fn currency_code(s: &str) -> Option<String> {
    let mut chars = s.chars();
    if let (Some(c), None) = (chars.next(), chars.next()) {
        return symbol_code(c).map(str::to_string);
    }
    (s.len() == 3 && s.chars().all(|c| c.is_ascii_alphabetic())).then(|| s.to_ascii_uppercase())
}

// ---------------------------------------------------------------------------
// Rates
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub struct Rates {
    /// Units of each currency per 1 USD.
    pub per_usd: HashMap<String, f64>,
    /// When the provider last updated these rates (unix seconds).
    pub updated: u64,
}

impl Rates {
    /// Returns `(converted amount, rate for 1 unit of `from`)`.
    pub fn convert(&self, q: &Query) -> Option<(f64, f64)> {
        let from = self.per_usd.get(&q.from)?;
        let to = self.per_usd.get(&q.to)?;
        let rate = to / from;
        Some((q.amount * rate, rate))
    }
}

/// Minimal parser for the provider's JSON: reads the flat `"rates": {"EUR": 0.92, ...}`
/// object and `time_last_update_unix`. Avoids pulling in a JSON dependency.
pub fn parse_rates_json(json: &str) -> Option<Rates> {
    let start = json.find("\"rates\"")?;
    let body = &json[start..];
    let body = &body[body.find('{')? + 1..];
    let body = &body[..body.find('}')?];
    let per_usd: HashMap<String, f64> = body
        .split(',')
        .filter_map(|pair| {
            let (k, v) = pair.split_once(':')?;
            let code = k.trim().trim_matches('"');
            let rate: f64 = v.trim().parse().ok()?;
            (code.len() == 3 && rate > 0.0).then(|| (code.to_string(), rate))
        })
        .collect();
    if per_usd.is_empty() {
        return None;
    }
    let updated = json
        .find("\"time_last_update_unix\"")
        .and_then(|i| {
            let rest = json[i..].split_once(':')?.1.trim_start();
            let end = rest.find(|c: char| !c.is_ascii_digit()).unwrap_or(rest.len());
            rest[..end].parse().ok()
        })
        .unwrap_or(0);
    Some(Rates { per_usd, updated })
}

// ---------------------------------------------------------------------------
// Formatting
// ---------------------------------------------------------------------------

/// `(display, plain)`: e.g. `("1,234.56", "1234.56")`. Small values keep ~4 significant digits.
pub fn format_money(x: f64) -> (String, String) {
    let decimals = if x == 0.0 || x.abs() >= 1.0 {
        2
    } else {
        ((-x.abs().log10()).ceil() as usize + 3).min(8)
    };
    let plain = format!("{x:.decimals$}");
    (group_thousands(&plain), plain)
}

/// Exchange rate with up to 4 decimals (more for tiny rates), trailing zeros trimmed.
pub fn format_rate(x: f64) -> String {
    let decimals = if x >= 1.0 { 4 } else { ((-x.log10()).ceil() as usize + 3).min(10) };
    let s = format!("{x:.decimals$}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    group_thousands(s)
}

fn group_thousands(s: &str) -> String {
    let (sign, s) = s.strip_prefix('-').map_or(("", s), |r| ("-", r));
    let (int, frac) = s.split_once('.').map_or((s, None), |(i, f)| (i, Some(f)));
    let mut out = String::new();
    for (i, c) in int.chars().enumerate() {
        if i > 0 && (int.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    match frac {
        Some(f) => format!("{sign}{out}.{f}"),
        None => format!("{sign}{out}"),
    }
}

pub fn format_age(secs: u64) -> String {
    match secs {
        0..60 => "just now".into(),
        60..3600 => format!("{}m ago", secs / 60),
        3600..86400 => format!("{}h ago", secs / 3600),
        _ => format!("{}d ago", secs / 86400),
    }
}

pub fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

// ---------------------------------------------------------------------------
// Rate store: disk cache + fetch bookkeeping (main thread only)
// ---------------------------------------------------------------------------

#[derive(Debug, Default)]
pub struct RateStore {
    pub rates: Option<Rates>,
    fetched_at: u64,
    pub fetching: bool,
    pub last_error: Option<(u64, String)>,
    cache_file: Option<PathBuf>,
}

impl RateStore {
    pub fn default_file() -> Option<PathBuf> {
        let home = std::env::var_os("HOME")?;
        Some(PathBuf::from(home).join("Library/Application Support/RoLauncher/rates.json"))
    }

    pub fn load(cache_file: Option<PathBuf>) -> Self {
        let mut store = Self { cache_file, ..Default::default() };
        if let Some(file) = &store.cache_file
            && let Ok(json) = std::fs::read_to_string(file)
        {
            store.rates = parse_rates_json(&json);
            store.fetched_at = mtime(file).unwrap_or(0);
        }
        store
    }

    pub fn needs_refresh(&self, now: u64) -> bool {
        let stale = self.rates.is_none() || now.saturating_sub(self.fetched_at) > MAX_AGE_SECS;
        let backing_off = self.last_error.as_ref().is_some_and(|(t, _)| now.saturating_sub(*t) < RETRY_SECS);
        stale && !self.fetching && !backing_off
    }

    /// Applies a finished fetch. On success the raw JSON is cached to disk.
    pub fn finish(&mut self, result: Result<String, String>, now: u64) {
        self.fetching = false;
        let parsed = result.and_then(|json| parse_rates_json(&json).map(|r| (r, json)).ok_or("unexpected response".into()));
        match parsed {
            Ok((rates, json)) => {
                self.rates = Some(rates);
                self.fetched_at = now;
                self.last_error = None;
                if let Some(file) = &self.cache_file {
                    let _ = file.parent().map(std::fs::create_dir_all);
                    if let Err(err) = std::fs::write(file, json) {
                        eprintln!("rolauncher: failed to cache rates: {err}");
                    }
                }
            }
            Err(err) => {
                eprintln!("rolauncher: exchange-rate fetch failed: {err}");
                self.last_error = Some((now, err));
            }
        }
    }
}

fn mtime(path: &Path) -> Option<u64> {
    let modified = std::fs::metadata(path).ok()?.modified().ok()?;
    Some(modified.duration_since(UNIX_EPOCH).ok()?.as_secs())
}

// ---------------------------------------------------------------------------
// Background fetch, delivered back on the main thread via libdispatch.
// ---------------------------------------------------------------------------

unsafe extern "C" {
    static _dispatch_main_q: c_void;
    fn dispatch_async_f(queue: *const c_void, context: *mut c_void, work: extern "C" fn(*mut c_void));
}

type Handler = Box<dyn Fn(Result<String, String>)>;

thread_local! {
    static ON_FETCHED: std::cell::RefCell<Option<Handler>> = const { std::cell::RefCell::new(None) };
}

/// Sets the main-thread callback that receives fetch results.
pub fn set_fetch_handler(handler: impl Fn(Result<String, String>) + 'static) {
    ON_FETCHED.with(|h| *h.borrow_mut() = Some(Box::new(handler)));
}

extern "C" fn deliver(context: *mut c_void) {
    let result = unsafe { Box::from_raw(context.cast::<Result<String, String>>()) };
    ON_FETCHED.with(|h| {
        if let Some(h) = h.borrow().as_ref() {
            h(*result);
        }
    });
}

/// Downloads the rates on a background thread; the result arrives via the fetch handler.
pub fn spawn_fetch() {
    std::thread::spawn(|| {
        let result = objc2::rc::autoreleasepool(|_| {
            use objc2_foundation::{NSData, NSString, NSURL};
            let url = NSURL::URLWithString(&NSString::from_str(RATES_URL)).ok_or("bad URL")?;
            let data = NSData::dataWithContentsOfURL(&url).ok_or("network request failed")?;
            String::from_utf8(data.to_vec()).map_err(|_| "response is not UTF-8".to_string())
        });
        let context = Box::into_raw(Box::new(result.map_err(|e: String| e)));
        unsafe { dispatch_async_f(&raw const _dispatch_main_q, context.cast(), deliver) };
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn q(amount: f64, from: &str, to: &str) -> Option<Query> {
        Some(Query { amount, from: from.into(), to: to.into() })
    }

    #[test]
    fn parses_queries() {
        assert_eq!(parse_query("100 USD to EUR"), q(100.0, "USD", "EUR"));
        assert_eq!(parse_query("100usd in eur"), q(100.0, "USD", "EUR"));
        assert_eq!(parse_query(" 1,250.5 gbp = jpy "), q(1250.5, "GBP", "JPY"));
        assert_eq!(parse_query("$100 to €"), q(100.0, "USD", "EUR"));
        assert_eq!(parse_query("100€ -> usd"), q(100.0, "EUR", "USD"));
        assert_eq!(parse_query("2*50 USD into CHF"), q(100.0, "USD", "CHF"));
    }

    #[test]
    fn rejects_non_currency_queries() {
        for s in ["safari", "100 USD", "USD to EUR", "100 dollars to EUR", "100 USD to", "go to app", "2+2"] {
            assert_eq!(parse_query(s), None, "{s:?}");
        }
    }

    #[test]
    fn parses_provider_json_and_converts() {
        let json = r#"{"result":"success","time_last_update_unix":1763510551,"base_code":"USD",
            "rates":{"USD":1,"EUR":0.8,"JPY":150.5,"BAD":"x"}}"#;
        let rates = parse_rates_json(json).unwrap();
        assert_eq!(rates.updated, 1763510551);
        assert_eq!(rates.per_usd.len(), 3);
        let (value, rate) = rates.convert(&Query { amount: 100.0, from: "EUR".into(), to: "USD".into() }).unwrap();
        assert!((value - 125.0).abs() < 1e-9 && (rate - 1.25).abs() < 1e-9);
        assert!(rates.convert(&Query { amount: 1.0, from: "USD".into(), to: "XYZ".into() }).is_none());
        assert!(parse_rates_json("{\"result\":\"error\"}").is_none());
    }

    #[test]
    fn formats_amounts() {
        assert_eq!(format_money(1234567.891), ("1,234,567.89".into(), "1234567.89".into()));
        assert_eq!(format_money(92.1), ("92.10".into(), "92.10".into()));
        assert_eq!(format_money(0.0001234).1, "0.0001234");
        assert_eq!(format_money(-1500.0).0, "-1,500.00");
        assert_eq!(format_rate(0.9213), "0.9213");
        assert_eq!(format_rate(151.23), "151.23");
        assert_eq!(format_age(7200), "2h ago");
    }

    #[test]
    fn store_refresh_policy() {
        let mut store = RateStore::default();
        assert!(store.needs_refresh(1000));
        store.fetching = true;
        assert!(!store.needs_refresh(1000));
        store.finish(Err("offline".into()), 1000);
        assert!(!store.needs_refresh(1030)); // backing off
        assert!(store.needs_refresh(1100));
        store.finish(Ok(r#"{"rates":{"USD":1,"EUR":0.9}}"#.into()), 2000);
        assert!(!store.needs_refresh(2000 + 3600));
        assert!(store.needs_refresh(2000 + MAX_AGE_SECS + 1));
    }
}
