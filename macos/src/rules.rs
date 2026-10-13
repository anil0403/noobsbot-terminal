//! Situation rules: `when` these things are true, use `theme`.

use serde::Deserialize;

use crate::context::Context;

#[derive(Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Rule {
    #[serde(default)]
    pub when: When,
    pub theme: String,
}

/// All given conditions must hold. Text conditions take a glob (`*`, `?`) or a list of them.
#[derive(Debug, Deserialize, PartialEq, Default)]
#[serde(default, deny_unknown_fields)]
pub struct When {
    pub ssh_host: Option<Patterns>,
    pub user: Option<Patterns>,
    pub git_branch: Option<Patterns>,
    pub cwd: Option<Patterns>,
    pub process: Option<Patterns>,
    pub hours: Option<Hours>,
}

#[derive(Debug, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum Patterns {
    One(String),
    Many(Vec<String>),
}

impl Patterns {
    fn matches(&self, text: Option<&str>) -> bool {
        let Some(text) = text else { return false };
        let one = |p: &String| glob(&expand_home(p), text);
        match self {
            Patterns::One(p) => one(p),
            Patterns::Many(ps) => ps.iter().any(one),
        }
    }
}

/// "20-6": from 20:00 up to (not including) 06:00, wrapping past midnight.
#[derive(Debug, Deserialize, PartialEq, Clone, Copy)]
#[serde(try_from = "String")]
pub struct Hours {
    start: u32,
    end: u32,
}

impl TryFrom<String> for Hours {
    type Error = String;
    fn try_from(s: String) -> Result<Self, String> {
        let err = || format!("hours must look like \"20-6\", got {s:?}");
        let (a, b) = s.split_once('-').ok_or_else(err)?;
        let (start, end) = (a.trim().parse().map_err(|_| err())?, b.trim().parse().map_err(|_| err())?);
        if start > 24 || end > 24 {
            return Err(err());
        }
        Ok(Hours { start, end })
    }
}

impl Hours {
    fn contains(&self, hour: u32) -> bool {
        if self.start <= self.end { (self.start..self.end).contains(&hour) } else { hour >= self.start || hour < self.end }
    }
}

impl When {
    fn matches(&self, ctx: &Context) -> bool {
        let cwd = ctx.cwd.as_ref().and_then(|p| p.to_str());
        let check = |p: &Option<Patterns>, value: Option<&str>| p.as_ref().is_none_or(|p| p.matches(value));
        check(&self.ssh_host, ctx.ssh_host.as_deref())
            && check(&self.user, Some(&ctx.user))
            && check(&self.git_branch, ctx.git_branch.as_deref())
            && check(&self.cwd, cwd)
            && check(&self.process, Some(&ctx.process))
            && self.hours.is_none_or(|h| h.contains(ctx.hour))
    }

    pub fn uses_hours(&self) -> bool {
        self.hours.is_some()
    }
}

/// First rule whose conditions all hold.
pub fn pick<'a>(rules: &'a [Rule], ctx: &Context) -> Option<&'a Rule> {
    rules.iter().find(|r| r.when.matches(ctx))
}

fn expand_home(pattern: &str) -> String {
    match pattern.strip_prefix('~') {
        Some(rest) => format!("{}{rest}", std::env::var("HOME").unwrap_or_default()),
        None => pattern.to_owned(),
    }
}

/// `*` matches any run of characters (including `/`), `?` exactly one.
fn glob(pattern: &str, text: &str) -> bool {
    let (p, t): (Vec<char>, Vec<char>) = (pattern.chars().collect(), text.chars().collect());
    let (mut pi, mut ti) = (0, 0);
    let mut backtrack: Option<(usize, usize)> = None;
    while ti < t.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == t[ti]) {
            pi += 1;
            ti += 1;
        } else if pi < p.len() && p[pi] == '*' {
            backtrack = Some((pi, ti));
            pi += 1;
        } else if let Some((bp, bt)) = backtrack {
            // Let the last `*` swallow one more character and retry.
            pi = bp + 1;
            ti = bt + 1;
            backtrack = Some((bp, bt + 1));
        } else {
            return false;
        }
    }
    p[pi..].iter().all(|&c| c == '*')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn globs() {
        assert!(glob("prod-*", "prod-db"));
        assert!(glob("*prod*", "eu-prod-1"));
        assert!(!glob("prod-*", "staging"));
        assert!(glob("web?", "web1"));
        assert!(!glob("web?", "web12"));
        assert!(glob("*", ""));
        assert!(glob("a*b*c", "aXXbYYc"));
        assert!(!glob("a*b*c", "aXXbYY"));
        assert!(glob("/home/*/work/*", "/home/devil/work/x/y"));
    }

    #[test]
    fn hours_wrap_midnight() {
        let night = Hours::try_from("20-6".to_string()).unwrap();
        assert!(night.contains(23) && night.contains(0) && night.contains(5));
        assert!(!night.contains(6) && !night.contains(12));
        let day = Hours::try_from("9-17".to_string()).unwrap();
        assert!(day.contains(9) && !day.contains(17));
        assert!(Hours::try_from("late".to_string()).is_err());
    }

    #[test]
    fn picks_first_matching_rule() {
        let rules: Vec<Rule> = toml::from_str::<std::collections::HashMap<String, Vec<Rule>>>(
            r#"
            [[rules]]
            when = { ssh_host = ["prod-*", "*.prod.example.com"] }
            theme = "danger"
            [[rules]]
            when = { user = "root" }
            theme = "root"
            [[rules]]
            when = { git_branch = ["main", "master"], hours = "9-17" }
            theme = "careful"
            "#,
        )
        .unwrap()
        .remove("rules")
        .unwrap();
        let ctx = |f: &dyn Fn(&mut Context)| {
            let mut c = Context { user: "devil".into(), hour: 12, ..Default::default() };
            f(&mut c);
            c
        };
        let theme = |c: Context| pick(&rules, &c).map(|r| r.theme.as_str());
        assert_eq!(theme(ctx(&|_| {})), None);
        assert_eq!(theme(ctx(&|c| c.ssh_host = Some("prod-db".into()))), Some("danger"));
        assert_eq!(theme(ctx(&|c| c.ssh_host = Some("a.prod.example.com".into()))), Some("danger"));
        assert_eq!(theme(ctx(&|c| c.ssh_host = Some("staging".into()))), None);
        assert_eq!(theme(ctx(&|c| c.user = "root".into())), Some("root"));
        // first match wins
        assert_eq!(
            theme(ctx(&|c| {
                c.user = "root".into();
                c.ssh_host = Some("prod-1".into())
            })),
            Some("danger")
        );
        // every condition in a rule must hold
        assert_eq!(theme(ctx(&|c| c.git_branch = Some("main".into()))), Some("careful"));
        assert_eq!(
            theme(ctx(&|c| {
                c.git_branch = Some("main".into());
                c.hour = 20
            })),
            None
        );
    }
}
