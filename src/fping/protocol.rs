use regex::Regex;
use std::time::Duration;

#[allow(dead_code)]
pub const LABEL_NAMES: [&str; 2] = ["target", "addr"];

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

impl<S: Copy> Ping<S> {
    pub fn labels(&self) -> [S; 2] {
        [self.target, self.addr]
    }
}
#[derive(Debug, PartialEq)]
pub struct SentReceivedSummary<S> {
    pub target: S,
    pub addr: S,
    pub sent: u32,
    pub received: u32,
}

impl<S: Copy> SentReceivedSummary<S> {
    pub fn labels(&self) -> [S; 2] {
        [self.target, self.addr]
    }
}

#[derive(Debug, PartialEq)]
pub enum Control<S> {
    IcmpError { target: S, addr: S, error: S },
    FpingError { target: S, message: S },
    BlankLine,
    SummaryLocalTime,
    TargetSummary(SentReceivedSummary<S>),
    Unhandled(S),
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

    fn parse_fping_error(raw: &'t str) -> Option<Self> {
        lazy_static! {
            static ref FPING_ERROR: Regex = Regex::new(
                r"(?x)
                ^(?P<target>[^:]+):
                \ (?P<msg>.*)$
            "
            )
            .unwrap();
        }

        let caps: regex::Captures = FPING_ERROR.captures(raw)?;
        Some(Control::FpingError {
            target: caps.name("target")?.as_str(),
            message: caps.name("msg")?.as_str(),
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
        Some(Control::TargetSummary(SentReceivedSummary {
            target: caps.name("target")?.as_str(),
            addr: caps.name("addr")?.as_str(),
            received: caps.name("rcv")?.as_str().parse().ok()?,
            sent: caps.name("xmt")?.as_str().parse().ok()?,
        }))
    }

    pub fn parse<S: AsRef<str> + ?Sized>(raw: &'t S) -> Self {
        #[inline]
        fn wrap_option<T, E: Copy>(
            try_fn: impl FnOnce(E) -> Option<T>,
        ) -> impl FnOnce(E) -> Result<T, E> {
            |value| try_fn(value).ok_or(value)
        }

        Err(raw.as_ref())
            .or_else(wrap_option(|x: &str| {
                if x.is_empty() {
                    //TODO: check whether an empty line is printed anywhere else....
                    Some(Control::BlankLine)
                } else if x.starts_with('[') && x.ends_with(']') {
                    Some(Control::SummaryLocalTime)
                } else {
                    None
                }
            }))
            .or_else(wrap_option(Self::parse_icmp_error))
            .or_else(wrap_option(Self::parse_status_line))
            .or_else(wrap_option(Self::parse_fping_error))
            .unwrap_or_else(Control::Unhandled)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_lines<S, O>(lines: impl IntoIterator<Item = S>, parse: impl Fn(S) -> O) -> Vec<O> {
        lines.into_iter().map(parse).collect()
    }

    #[test]
    fn parse_response() {
        assert_eq!(
            Ping::parse("[1611765997.71135] localhost (127.0.0.1) : [9], 64 bytes, 0.029 ms (0.040 avg, 0% loss)"),
            Some(Ping {
                timestamp: Duration::from_secs_f64("1611765997.71135".parse().unwrap()),
                target: "localhost",
                addr: "127.0.0.1",
                seq: 9,
                result: Some(Duration::from_micros(29)),
            })
        );

        assert_eq!(Ping::parse(""), None);
    }

    #[test]
    fn parse_signal_summary() {
        assert_eq!(parse_lines(
            "\n\
            [16:55:13]\n\
            dns.google (8.8.4.4) : xmt/rcv/%loss = 104/104/0%, min/avg/max = 10.5/18.6/77.9\n\
            localhost (127.0.0.1) : xmt/rcv/%loss = 104/104/0%, min/avg/max = 0.025/0.063/0.189\n\
            8.8.8.7 (8.8.8.7) : xmt/rcv/%loss = 0/0/0%\n\
            ipv6.google.com (2a00:1450:400e:806::200e) : xmt/rcv/%loss = 104/0/100%\n\
            ns1.webtraf.com.au (103.224.162.40) : xmt/rcv/%loss = 104/104/0%, min/avg/max = 338/346/461"
            .split('\n'),
            Control::parse,
        ), &[
            Control::BlankLine,
            Control::SummaryLocalTime,
            Control::TargetSummary(SentReceivedSummary {
                target: "dns.google",
                addr: "8.8.4.4",
                sent: 104,
                received: 104
            }),
            Control::TargetSummary(SentReceivedSummary  {
                target: "localhost",
                addr: "127.0.0.1",
                sent: 104,
                received: 104
            }),
            Control::TargetSummary(SentReceivedSummary  {
                target: "8.8.8.7",
                addr: "8.8.8.7",
                sent: 0,
                received: 0
            }),
            Control::TargetSummary(SentReceivedSummary  {
                target: "ipv6.google.com",
                addr: "2a00:1450:400e:806::200e",
                sent: 104,
                received: 0
            }),
            Control::TargetSummary(SentReceivedSummary  {
                target: "ns1.webtraf.com.au",
                addr: "103.224.162.40",
                sent: 104,
                received: 104
            }),
        ]);
    }
}
