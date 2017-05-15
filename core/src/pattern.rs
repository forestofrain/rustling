use std::cmp::{PartialOrd, Ordering};
use std::rc;

use smallvec::SmallVec;

use errors::*;
use {AttemptFrom, Sym, Node, ParsedNode, SendSyncPhantomData, Stash};

pub fn alphanumeric_class(c: char) -> char {
    if c.is_alphanumeric() { 'A' } else { c }
}

pub fn detailed_class(c: char) -> char {
    if c.is_uppercase() {
        'u'
    } else if c.is_lowercase() {
        'l'
    } else if c.is_digit(10) {
        'd'
    } else {
        c
    }
}

fn separated_substring<CharClass>(sentence: &str, range: Range, char_class: &CharClass) -> bool
    where CharClass: Fn(char) -> char
{
    let first_mine = sentence[range.0..range.1]
        .chars()
        .next()
        .map(char_class); // Some(c)
    let last_mine = sentence[range.0..range.1]
        .chars()
        .next_back()
        .map(char_class); //Some(c)
    let last_before = sentence[..range.0].chars().next_back().map(char_class); // Option(c)
    let first_after = sentence[range.1..].chars().next().map(char_class); // Option(c)

    first_mine != last_before && last_mine != first_after
}

/// Represent a semi-inclusive range of position, in bytes, in the matched
/// sentence.
#[derive(PartialEq,Clone,Debug,Copy,Hash,Eq)]
pub struct Range(pub usize, pub usize);

impl Range {
    pub fn intersects(&self, other: &Self) -> bool {
        self.partial_cmp(other).is_none() && (self.1 >= other.0 && other.1 >= self.0)
    }
}

impl PartialOrd for Range {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        if self == other {
            Some(Ordering::Equal)
        } else if self.0 <= other.0 && other.1 <= self.1 {
            Some(Ordering::Greater)
        } else if other.0 <= self.0 && self.1 <= other.1 {
            Some(Ordering::Less)
        } else {
            None
        }
    }
}

pub trait Match: Clone {
    fn range(&self) -> Range;
    fn to_node(&self) -> rc::Rc<Node>;
}

impl<V: Clone> Match for ParsedNode<V> {
    fn range(&self) -> Range {
        self.root_node.range
    }

    fn to_node(&self) -> rc::Rc<Node> {
        self.root_node.clone()
    }
}

#[derive(Clone,Debug,PartialEq)]
pub struct Text {
    pub groups: SmallVec<[Range; 4]>,
    range: Range,
    pattern_sym: Sym,
}

impl Text {
    pub fn new(groups: SmallVec<[Range; 4]>, range: Range, pattern_sym: Sym) -> Text {
        Text {
            groups: groups,
            range: range,
            pattern_sym: pattern_sym,
        }
    }
}

impl Match for Text {
    fn range(&self) -> Range {
        self.range
    }

    fn to_node(&self) -> rc::Rc<Node> {
        rc::Rc::new(Node {
                        rule_sym: self.pattern_sym,
                        range: self.range(),
                        children: SmallVec::new(),
                    })
    }
}

pub type PredicateMatches<M> = Vec<M>;

pub trait Pattern<StashValue: Clone>: Send + Sync {
    type M: Match;
    fn predicate(&self,
                 stash: &Stash<StashValue>,
                 sentence: &str)
                 -> CoreResult<PredicateMatches<Self::M>>;
}


pub struct TextPattern<StashValue: Clone>(::regex::Regex, Sym, SendSyncPhantomData<StashValue>);

impl<StashValue: Clone> TextPattern<StashValue> {
    pub fn new(regex: ::regex::Regex, sym: Sym) -> TextPattern<StashValue> {
        TextPattern(regex, sym, SendSyncPhantomData::new())
    }
}

impl<StashValue: Clone> Pattern<StashValue> for TextPattern<StashValue> {
    type M = Text;
    fn predicate(&self,
                 _stash: &Stash<StashValue>,
                 sentence: &str)
                 -> CoreResult<PredicateMatches<Self::M>> {
        let mut results = PredicateMatches::new();
        for cap in self.0.captures_iter(&sentence) {
            let full = cap.get(0)
                .ok_or_else(|| {
                                format!("No capture for regexp {} in rule {:?} for sentence: {}",
                                        self.0,
                                        self.1,
                                        sentence)
                            })?;
            let full_range = Range(full.start(), full.end());
            if !separated_substring(sentence, full_range, &detailed_class) {
                continue;
            }
            let mut groups = SmallVec::new();
            for (ix, group) in cap.iter().enumerate() {
                let group = group.ok_or_else(|| {
                            format!("No capture for regexp {} in rule {:?}, group number {} in \
                                     capture: {}",
                                    self.0,
                                    self.1,
                                    ix,
                                    full.as_str())
                        })?;
                let range = Range(group.start(), group.end());
                groups.push(range);
            }
            results.push(Text {
                             groups: groups,
                             range: full_range,
                             pattern_sym: self.1,
                         })
        }

        Ok(results)
    }
}

pub struct TextNegLHPattern<StashValue: Clone> {
    pattern: ::regex::Regex,
    neg_look_ahead: ::regex::Regex,
    pattern_sym: Sym,
    _phantom: SendSyncPhantomData<StashValue>,
}

impl<StashValue: Clone> TextNegLHPattern<StashValue> {
    pub fn new(pattern: ::regex::Regex,
               neg_look_ahead: ::regex::Regex,
               pattern_sym: Sym)
               -> TextNegLHPattern<StashValue> {
        TextNegLHPattern {
            pattern: pattern,
            neg_look_ahead: neg_look_ahead,
            pattern_sym: pattern_sym,
            _phantom: SendSyncPhantomData::new(),
        }
    }
}

impl<StashValue: Clone> Pattern<StashValue> for TextNegLHPattern<StashValue> {
    type M = Text;
    fn predicate(&self,
                 _stash: &Stash<StashValue>,
                 sentence: &str)
                 -> CoreResult<PredicateMatches<Text>> {
        let mut results = PredicateMatches::new();
        for cap in self.pattern.captures_iter(&sentence) {
            let full = cap.get(0)
                .ok_or_else(|| {
                                format!("No capture for regexp {} in rule {:?} for sentence: {}",
                                        self.pattern,
                                        self.pattern_sym,
                                        sentence)
                            })?;
            let full_range = Range(full.start(), full.end());
            if !separated_substring(sentence, full_range, &detailed_class) {
                continue;
            }
            if let Some(mat) = self.neg_look_ahead.find(&sentence[full.end()..]) {
                if mat.start() == 0 {
                    continue;
                }
            }
            let mut groups = SmallVec::new();
            for (ix, group) in cap.iter().enumerate() {
                let group = group.ok_or_else(|| {
                            format!("No capture for regexp {} in rule {:?}, group number {} in \
                                     capture: {}",
                                    self.pattern,
                                    self.pattern_sym,
                                    ix,
                                    full.as_str())
                        })?;
                let range = Range(group.start(), group.end());
                groups.push(range);
            }
            results.push(Text {
                             groups: groups,
                             range: full_range,
                             pattern_sym: self.pattern_sym,
                         })
        }

        Ok(results)
    }
}

pub type AnyNodePattern<V> = FilterNodePattern<V>;

pub struct FilterNodePattern<V>
    where V: Clone
{
    predicates: Vec<Box<Fn(&V) -> bool + Send + Sync>>,
    _phantom: SendSyncPhantomData<V>,
}

impl<V: Clone> AnyNodePattern<V> {
    pub fn new() -> AnyNodePattern<V> {
        FilterNodePattern {
            predicates: vec![],
            _phantom: SendSyncPhantomData::new(),
        }
    }
}

impl<V> FilterNodePattern<V>
    where V: Clone
{
    pub fn filter(predicates: Vec<Box<Fn(&V) -> bool + Sync + Send>>) -> FilterNodePattern<V> {
        FilterNodePattern {
            predicates: predicates,
            _phantom: SendSyncPhantomData::new(),
        }
    }
}

impl<StashValue, V> Pattern<StashValue> for FilterNodePattern<V>
    where StashValue: Clone,
          V: AttemptFrom<StashValue> + Clone
{
    type M = ParsedNode<V>;
    fn predicate(&self,
                 stash: &Stash<StashValue>,
                 _sentence: &str)
                 -> CoreResult<PredicateMatches<ParsedNode<V>>> {
        Ok(stash
               .iter()
               .filter_map(|it| if let Some(v) = V::attempt_from(it.value.clone()) {
                               if self.predicates.iter().all(|predicate| (predicate)(&v)) {
                                   Some(ParsedNode::new(it.root_node.rule_sym,
                                                        v,
                                                        it.range(),
                                                        it.root_node.children.clone()))
                               } else {
                                   None
                               }
                           } else {
                               None
                           })
               .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_separated_substring() {
        let an = |c: char| if c.is_alphanumeric() { 'A' } else { c };
        assert_eq!(true, separated_substring("abc def ret", Range(4, 7), &an)); // "def"
        assert_eq!(false, separated_substring("abc def ret", Range(2, 8), &an)); // "c def r"
        assert_eq!(false,
                   separated_substring("abc def123 ret", Range(4, 7), &an)); // "def"
        assert_eq!(true, separated_substring("def123 ret", Range(0, 6), &an)); // "def123"
        assert_eq!(false, separated_substring("def123 ret", Range(0, 3), &an)); // "def"
        assert_eq!(true, separated_substring("ret def", Range(4, 7), &an)); // "def"
        assert_eq!(false, separated_substring("ret 123def", Range(7, 10), &an)); // "def"
        assert_eq!(false, separated_substring("aéc def ret", Range(3, 9), &an)); // "c def r"
        assert_eq!(false, separated_substring("aec def rét", Range(2, 8), &an)); // "c def r"
        assert_eq!(false, separated_substring("aec déf ret", Range(2, 9), &an)); // "c déf r"
        assert_eq!(false, separated_substring("aeç def ret", Range(2, 9), &an)); // "ç def r"
        assert_eq!(true, separated_substring("aeç def ret", Range(4, 8), &an)); // " def "
    }

    macro_rules! svec4 {
        ($($item:expr),*) => { {
            let mut v = ::smallvec::SmallVec::<[_;4]>::new();
            $( v.push($item); )*
            v
        }
        }
    }

    #[test]
    fn test_regex_separated_string() {
        let stash = vec![];
        let pat: TextPattern<usize> = TextPattern::new(::regex::Regex::new("a+").unwrap(), Sym(0));
        assert_eq!(vec![Text::new(svec4!(Range(0, 3)), Range(0, 3), Sym(0))],
                   pat.predicate(&stash, "aaa").unwrap());
        assert_eq!(vec![Text::new(svec4!(Range(0, 3)), Range(0, 3), Sym(0))],
                   pat.predicate(&stash, "aaa bbb").unwrap());
        assert_eq!(vec![Text::new(svec4!(Range(4, 7)), Range(4, 7), Sym(0))],
                   pat.predicate(&stash, "bbb aaa").unwrap());
        assert_eq!(Vec::<Text>::new(), pat.predicate(&stash, "baaa").unwrap());
        assert_eq!(Vec::<Text>::new(), pat.predicate(&stash, "aaab").unwrap());
        assert_eq!(Vec::<Text>::new(), pat.predicate(&stash, "aaaé").unwrap());
        assert_eq!(Vec::<Text>::new(), pat.predicate(&stash, "éaaa").unwrap());
        assert_eq!(vec![Text::new(svec4!(Range(1, 4)), Range(1, 4), Sym(0))],
                   pat.predicate(&stash, "1aaa").unwrap());
        assert_eq!(vec![Text::new(svec4!(Range(0, 3)), Range(0, 3), Sym(0))],
                   pat.predicate(&stash, "aaa1").unwrap());
        assert_eq!(vec![Text::new(svec4!(Range(0, 3)), Range(0, 3), Sym(0))],
                   pat.predicate(&stash, "aaa-toto").unwrap());
    }
}
