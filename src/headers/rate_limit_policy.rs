use super::{ParseError, quota::QuotaUnit, sf};
use sfparse::{Parser, Value};
use std::{
    borrow::Cow,
    fmt::{self, Display},
    time::Duration,
};
use trillium::Headers;

const POLICY_HEADER: &str = "RateLimit-Policy";

/// A single quota policy — one item of the `RateLimit-Policy` header field's Structured Field
/// List.
///
/// A policy advertises a server's standing quota allocation: a required quota `q`, an optional
/// time window `w`, an optional [`QuotaUnit`] `qu` (defaulting to requests), and an optional
/// partition key `pk`. The type is deliberately unopinionated — it represents the full range the
/// field can express, including windowless and non-request-unit policies the limiter in this
/// crate could never itself enforce.
///
/// Parse the full header (lists may span multiple field lines) with [`from_headers`] or a single
/// field value with [`parse_list`]; format one policy via its [`Display`] impl.
///
/// [`from_headers`]: RateLimitPolicy::from_headers
/// [`parse_list`]: RateLimitPolicy::parse_list
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RateLimitPolicy<'a> {
    name: Cow<'a, str>,
    quota: u64,
    unit: QuotaUnit<'a>,
    window: Option<Duration>,
    partition_key: Option<Cow<'a, str>>,
}

impl<'a> RateLimitPolicy<'a> {
    /// Builds a policy named `name` with quota `quota`, the default unit (requests), and no
    /// window or partition key.
    pub fn new(name: impl Into<Cow<'a, str>>, quota: u64) -> Self {
        Self {
            name: name.into(),
            quota,
            unit: QuotaUnit::Requests,
            window: None,
            partition_key: None,
        }
    }

    /// Sets the time window over which the quota is allocated (the `w` parameter).
    pub fn with_window(mut self, window: Duration) -> Self {
        self.window = Some(window);
        self
    }

    /// Sets the quota unit (the `qu` parameter).
    pub fn with_unit(mut self, unit: QuotaUnit<'a>) -> Self {
        self.unit = unit;
        self
    }

    /// Sets the partition key (the `pk` parameter), as its base64 wire form.
    pub fn with_partition_key(mut self, partition_key: impl Into<Cow<'a, str>>) -> Self {
        self.partition_key = Some(partition_key.into());
        self
    }

    /// The policy name — the identifier shared with the matching [`RateLimit`] service-limit item.
    ///
    /// [`RateLimit`]: crate::headers::RateLimit
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The quota allocated by this policy (the `q` parameter), in [units](Self::unit).
    pub fn quota(&self) -> u64 {
        self.quota
    }

    /// The unit the quota is measured in (the `qu` parameter; requests by default).
    pub fn unit(&self) -> &QuotaUnit<'a> {
        &self.unit
    }

    /// The time window the quota is allocated over (the `w` parameter), if advertised.
    pub fn window(&self) -> Option<Duration> {
        self.window
    }

    /// The partition key (the `pk` parameter), as its base64-encoded wire form, if present.
    pub fn partition_key(&self) -> Option<&str> {
        self.partition_key.as_deref()
    }

    /// Parses all policies from a single `RateLimit-Policy` field value (a Structured Field List
    /// of policy items). Returns [`ParseError`] if the value is not well-formed.
    pub fn parse_list(input: &'a str) -> Result<Vec<Self>, ParseError> {
        let mut parser = Parser::new(input.as_bytes());
        let mut policies = Vec::new();

        while let Some(value) = parser.parse_list().map_err(|_| ParseError)? {
            // A policy item's value must be a String (its name); anything else is out of spec, so
            // drain its parameters and skip it.
            let name = match value {
                Value::String { range, escape } => sf::string_value(input, range, escape),
                other => {
                    sf::skip_item(&mut parser, other)?;
                    continue;
                }
            };

            let mut quota = None;
            let mut unit = QuotaUnit::Requests;
            let mut window = None;
            let mut partition_key = None;

            while let Some((key, param)) = parser.parse_param().map_err(|_| ParseError)? {
                match (key, param) {
                    ("q", Value::Integer(value)) if value >= 0 => quota = Some(value as u64),
                    ("qu", Value::String { range, escape }) => {
                        unit = QuotaUnit::from_cow(sf::string_value(input, range, escape));
                    }
                    ("w", Value::Integer(value)) if value > 0 => {
                        window = Some(Duration::from_secs(value as u64));
                    }
                    ("pk", Value::ByteSeq(range)) => {
                        partition_key = Some(Cow::Borrowed(&input[range]));
                    }
                    _ => {}
                }
            }

            // The quota `q` parameter is required; an item without it is not a usable policy.
            if let Some(quota) = quota {
                policies.push(Self {
                    name,
                    quota,
                    unit,
                    window,
                    partition_key,
                });
            }
        }

        Ok(policies)
    }

    /// Parses every policy advertised across all `RateLimit-Policy` field lines in `headers`.
    pub fn from_headers(headers: &'a Headers) -> Result<Vec<Self>, ParseError> {
        let mut policies = Vec::new();
        if let Some(values) = headers.get_values(POLICY_HEADER) {
            for value in values.iter() {
                if let Some(value) = value.as_str() {
                    policies.extend(Self::parse_list(value)?);
                }
            }
        }
        Ok(policies)
    }

    /// Converts a borrowed policy into an owned `RateLimitPolicy<'static>`.
    pub fn into_owned(self) -> RateLimitPolicy<'static> {
        RateLimitPolicy {
            name: Cow::Owned(self.name.into_owned()),
            quota: self.quota,
            unit: self.unit.into_owned(),
            window: self.window,
            partition_key: self.partition_key.map(|pk| Cow::Owned(pk.into_owned())),
        }
    }
}

impl Display for RateLimitPolicy<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        sf::write_sf_string(f, &self.name)?;
        write!(f, ";q={}", self.quota)?;
        if !matches!(self.unit, QuotaUnit::Requests) {
            f.write_str(";qu=")?;
            sf::write_sf_string(f, self.unit.as_str())?;
        }
        if let Some(window) = self.window {
            write!(f, ";w={}", window.as_secs())?;
        }
        if let Some(partition_key) = &self.partition_key {
            write!(f, ";pk=:{partition_key}:")?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn one(input: &str) -> RateLimitPolicy<'_> {
        let mut policies = RateLimitPolicy::parse_list(input).unwrap();
        assert_eq!(
            policies.len(),
            1,
            "expected exactly one policy in {input:?}"
        );
        policies.pop().unwrap()
    }

    #[test]
    fn basic() {
        let policy = one(r#""default";q=100;w=10"#);
        assert_eq!(policy.name(), "default");
        assert_eq!(policy.quota(), 100);
        assert_eq!(policy.window(), Some(Duration::from_secs(10)));
        assert_eq!(policy.unit(), &QuotaUnit::Requests);
        assert_eq!(policy.partition_key(), None);
    }

    #[test]
    fn multiple_policies() {
        let policies =
            RateLimitPolicy::parse_list(r#""permin";q=50;w=60,"perhr";q=1000;w=3600"#).unwrap();
        assert_eq!(policies.len(), 2);
        assert_eq!(policies[0].name(), "permin");
        assert_eq!(policies[1].name(), "perhr");
        assert_eq!(policies[1].quota(), 1000);
    }

    #[test]
    fn unit_and_partition_key() {
        let policy = one(r#""peruser";q=65535;qu="content-bytes";w=10;pk=:sdfjLJUOUH==:"#);
        assert_eq!(policy.unit(), &QuotaUnit::ContentBytes);
        assert_eq!(policy.partition_key(), Some("sdfjLJUOUH=="));
    }

    #[test]
    fn window_is_optional() {
        let policy = one(r#""default";q=999;pk=:dHJpYWwxMjEzMjM=:"#);
        assert_eq!(policy.window(), None);
        assert_eq!(policy.partition_key(), Some("dHJpYWwxMjEzMjM="));
    }

    #[test]
    fn unknown_parameters_are_ignored() {
        let policy = one(r#""x";q=5;acme-burst=10"#);
        assert_eq!(policy.quota(), 5);
    }

    #[test]
    fn item_without_quota_is_skipped() {
        assert!(
            RateLimitPolicy::parse_list(r#""x";w=5"#)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn escaped_name() {
        let policy = one(r#""a\"b";q=1"#);
        assert_eq!(policy.name(), r#"a"b"#);
    }

    #[test]
    fn display_round_trips() {
        for input in [
            r#""default";q=100;w=10"#,
            r#""peruser";q=65535;qu="content-bytes";w=10;pk=:sdfjLJUOUH==:"#,
            r#""default";q=999;pk=:dHJpYWwxMjEzMjM=:"#,
        ] {
            let policy = one(input);
            let formatted = policy.to_string();
            assert_eq!(
                policy,
                one(&formatted),
                "round trip of {input:?} via {formatted:?}"
            );
        }
    }

    #[test]
    fn builder_formats() {
        let policy = RateLimitPolicy::new("default", 100).with_window(Duration::from_secs(60));
        assert_eq!(policy.to_string(), r#""default";q=100;w=60"#);
    }
}
