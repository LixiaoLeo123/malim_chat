/// Providers and models disagree about how private reasoning is framed, so every marker
/// form is normalised here before an answer is stored or streamed.

pub(crate) fn split_thinking(input: &str) -> (String, String) {
    let mut answer = String::new();
    let mut reasoning = String::new();
    let mut remaining = input;
    let mut in_think = false;
    while !remaining.is_empty() {
        match find_thinking_marker(remaining, in_think) {
            Some((index, marker_len)) => {
                if in_think {
                    reasoning.push_str(&remaining[..index]);
                } else {
                    answer.push_str(&remaining[..index]);
                }
                remaining = &remaining[index + marker_len..];
                in_think = !in_think;
            }
            None => {
                if in_think {
                    reasoning.push_str(remaining);
                } else {
                    answer.push_str(remaining);
                }
                break;
            }
        }
    }
    (answer, reasoning)
}

pub(crate) fn strip_thinking(input: &str) -> String {
    split_thinking(input).0
}

fn thinking_markers(in_think: bool) -> &'static [&'static str] {
    if in_think {
        &["</thinking>", "</think>"]
    } else {
        &["<thinking>", "<think>"]
    }
}

fn find_thinking_marker(input: &str, in_think: bool) -> Option<(usize, usize)> {
    thinking_markers(in_think)
        .iter()
        .filter_map(|marker| {
            input
                .as_bytes()
                .windows(marker.len())
                .position(|candidate| candidate.eq_ignore_ascii_case(marker.as_bytes()))
                .map(|index| (index, marker.len()))
        })
        .min_by_key(|(index, _)| *index)
}

fn thinking_marker_suffix_len(input: &str, in_think: bool) -> usize {
    thinking_markers(in_think)
        .iter()
        .flat_map(|marker| (1..marker.len()).rev().map(move |size| (marker, size)))
        .filter(|(marker, size)| {
            input.len() >= *size
                && input.as_bytes()[input.len() - *size..]
                    .eq_ignore_ascii_case(&marker.as_bytes()[..*size])
        })
        .map(|(_, size)| size)
        .max()
        .unwrap_or(0)
}

pub(crate) struct ThinkingStream {
    in_think: bool,
    pending: String,
}
impl ThinkingStream {
        pub(crate) fn new() -> Self {
        Self {
            in_think: false,
            pending: String::new(),
        }
    }
        pub(crate) fn push(&mut self, input: &str) -> Vec<(bool, String)> {
        self.pending.push_str(input);
        let mut output = Vec::new();
        loop {
            if let Some((index, marker_len)) = find_thinking_marker(&self.pending, self.in_think) {
                if index > 0 {
                    output.push((self.in_think, self.pending[..index].to_string()));
                }
                self.pending.drain(..index + marker_len);
                self.in_think = !self.in_think;
                continue;
            }
            let keep = thinking_marker_suffix_len(&self.pending, self.in_think);
            let safe = self.pending.len().saturating_sub(keep);
            if safe > 0 {
                output.push((self.in_think, self.pending[..safe].to_string()));
                self.pending.drain(..safe);
            }
            return output;
        }
    }
        pub(crate) fn finish(&mut self) -> Vec<(bool, String)> {
        if self.pending.is_empty() {
            vec![]
        } else {
            vec![(self.in_think, std::mem::take(&mut self.pending))]
        }
    }
}
