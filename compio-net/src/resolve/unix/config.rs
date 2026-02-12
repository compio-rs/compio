use std::{
    cmp,
    io,
    net::{IpAddr, Ipv4Addr, SocketAddr},
    time::Duration,
};

use logos::Logos;

#[derive(Debug, Clone)]
pub struct ResolvConf {
    pub nameservers: Vec<SocketAddr>,
    pub search: Vec<String>,
    pub ndots: u8,
    pub timeout: Duration,
    pub attempts: u8,
}

#[derive(Logos, Debug, PartialEq)]
#[logos(skip r"[ \t]+")]
enum Token<'a> {
    #[regex(r"#[^\n]*", allow_greedy = true)]
    Comment,

    #[token("nameserver")]
    Nameserver,

    #[token("search")]
    Search,

    #[token("domain")]
    Domain,

    #[token("options")]
    Options,

    #[regex(r"[^\s#]+", |lex| lex.slice())]
    Word(&'a str),

    #[token("\n")]
    Newline,
}

impl ResolvConf {
    pub fn load() -> io::Result<Self> {
        let content = std::fs::read_to_string("/etc/resolv.conf")?;
        Ok(Self::parse(&content))
    }

    pub fn parse(content: &str) -> Self {
        let mut nameservers = Vec::new();
        let mut search = Vec::new();
        let mut ndots = 1u8;
        let mut timeout = Duration::from_secs(5);
        let mut attempts = 2u8;

        let mut lexer = Token::lexer(content);

        while let Some(tok) = lexer.next() {
            let Ok(tok) = tok else { continue };
            match tok {
                Token::Nameserver => {
                    if let Some(word) = next_word(&mut lexer)
                        && let Ok(ip) = word.parse::<IpAddr>()
                    {
                        nameservers.push(SocketAddr::new(ip, 53));
                    }
                    skip_line(&mut lexer);
                }
                Token::Search => {
                    search.clear();
                    while let Some(word) = next_word(&mut lexer) {
                        search.push(word.to_string());
                    }
                }
                Token::Domain => {
                    search.clear();
                    if let Some(word) = next_word(&mut lexer) {
                        search.push(word.to_string());
                    }
                    skip_line(&mut lexer);
                }
                Token::Options => {
                    while let Some(opt) = next_word(&mut lexer) {
                        if let Some(v) = opt.strip_prefix("ndots:")
                            && let Ok(n) = v.parse::<u8>()
                        {
                            ndots = cmp::min(n, 15);
                        } else if let Some(v) = opt.strip_prefix("timeout:")
                            && let Ok(n) = v.parse::<u64>()
                        {
                            timeout = Duration::from_secs(cmp::min(n, 30));
                        } else if let Some(v) = opt.strip_prefix("attempts:")
                            && let Ok(n) = v.parse::<u8>()
                        {
                            attempts = cmp::min(n, 5);
                        }
                    }
                }
                Token::Comment | Token::Newline | Token::Word(_) => {}
            }
        }

        if nameservers.is_empty() {
            nameservers.push(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 53));
        }

        Self {
            nameservers,
            search,
            ndots,
            timeout,
            attempts,
        }
    }
}

fn next_word<'a>(lexer: &mut logos::Lexer<'a, Token<'a>>) -> Option<&'a str> {
    loop {
        let tok = lexer.next()?;
        match tok {
            Ok(Token::Word(w)) => return Some(w),
            Ok(Token::Newline | Token::Comment) | Err(_) => return None,
            _ => continue,
        }
    }
}

fn skip_line<'a>(lexer: &mut logos::Lexer<'a, Token<'a>>) {
    for tok in lexer.by_ref() {
        if matches!(tok, Ok(Token::Newline | Token::Comment)) {
            break;
        }
    }
}
