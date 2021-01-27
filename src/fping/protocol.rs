use regex::Regex;
use std::time::Duration;
#[derive(Debug, PartialEq)]
pub struct Ping<S> {
    pub timestamp: Duration,
    pub target: S,
    pub addr: S,
    pub seq: u64,
    pub result: Option<Duration>,
}

impl<'y> Ping<&'y str> {
    pub fn parse<S: AsRef<str> + ?Sized>(raw: &'y S) -> Option<Self> {
        lazy_static! {
            static ref FPING_LINE: Regex = Regex::new(
                r"(?x)
                    ^\[(?P<ts>[^\]]+)\]          # [1607718717.47230] 
                    \s(?P<id>.+?)                # dns.google
                    \s\((?P<addr>[^\)]+)\)\s+:   # (8.8.8.8)                       :
                    \s\[(?P<seq>\d+)\],          # [0], 
                    \s(?:
                        timed|                   # timed out 
                        \d+\sbytes,\s(?P<rtt>    # 64 bytes, 
                            [^\s]+               # 18.3 ms || 283 ms
                        )\s ms
                    )
                    .*$
                "
            )
            .unwrap();
        }

        fn millis_to_duration(time: f64) -> Duration {
            lazy_static! {
                static ref MILLISECOND: Duration = Duration::from_millis(1);
            }
            MILLISECOND.mul_f64(time)
        }

        let caps = FPING_LINE.captures(raw.as_ref())?;
        Some(Ping {
            timestamp: caps
                .name("ts")?
                .as_str()
                .parse()
                .map(Duration::from_secs_f64)
                .ok()?,
            target: caps.name("id")?.as_str(),
            addr: caps.name("addr")?.as_str(),
            seq: caps.name("seq")?.as_str().parse().ok()?,
            result: caps
                .name("rtt")
                .map_or_else(
                    || Ok(None),
                    |rtt| rtt.as_str().parse().map(millis_to_duration).map(Some),
                )
                .ok()?,
        })
    }
}

#[derive(Debug, PartialEq)]
pub enum Control<S> {
    IcmpError {
        target: S,
        addr: S,
        error: S,
    },
    StatusBegin,
    RandomLocalTime,
    StatusLine {
        target: S,
        addr: S,
        sent: u32,
        received: u32,
    },
}

impl<'t> Control<&'t str> {
    fn parse_icmp_error(raw: &'t str) -> Option<Self> {
        lazy_static! {
            static ref ICMP_ERROR: Regex = Regex::new(
                r"(?x)
                ^(?P<error>.+)
                \ from
                \ (?P<addr>[^\s]+)
                \ for\ ICMP\ Echo\ sent\ to
                \ (?P<target>.+)$
            "
            )
            .unwrap();
        }

        let caps: regex::Captures = ICMP_ERROR.captures(raw)?;
        Some(Control::IcmpError {
            error: caps.name("error")?.as_str(),
            addr: caps.name("addr")?.as_str(),
            target: caps.name("target")?.as_str(),
        })
    }

    fn parse_status_line(raw: &'t str) -> Option<Self> {
        lazy_static! {
            static ref STATUS_LINE: Regex = Regex::new(
                r"(?x)
                ^(?P<target>.+?)             # dns.google
                \ \((?P<addr>[^\)]+)\)\s+:   # (8.8.8.8)                       :
                \ [^\s]+\ =                  # xmt/rcv/%loss = 
                \ (?P<xmt>\d+)               # 1
                /(?P<rcv>\d+)                # /1
                .*$                          # /0%, min/avg/max = 16.3/16.3/16.3
            "
            )
            .unwrap();
        }

        let caps: regex::Captures = STATUS_LINE.captures(raw)?;
        Some(Control::StatusLine {
            target: caps.name("target")?.as_str(),
            addr: caps.name("addr")?.as_str(),
            received: caps.name("rcv")?.as_str().parse().ok()?,
            sent: caps.name("xmt")?.as_str().parse().ok()?,
        })
    }

    pub fn parse<S: AsRef<str> + ?Sized>(raw: &'t S) -> Option<Self> {
        #[inline]
        fn wrap_option<T, E: Copy>(
            try_fn: impl FnOnce(E) -> Option<T>,
        ) -> impl FnOnce(E) -> Result<T, E> {
            |value| try_fn(value).ok_or(value)
        }

        Err(raw.as_ref())
            .or_else(wrap_option(|x: &str| {
                if x.is_empty() {
                    Some(Control::StatusBegin)
                } else if x.starts_with('[') && x.ends_with(']') {
                    Some(Control::RandomLocalTime)
                } else {
                    None
                }
            }))
            .or_else(wrap_option(Self::parse_icmp_error))
            .or_else(wrap_option(Self::parse_status_line))
            .ok()
    }
}
