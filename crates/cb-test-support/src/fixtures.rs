use cb_context::RawEvent;

pub fn jsonl(events: &[RawEvent]) -> Result<String, serde_json::Error> {
    events
        .iter()
        .map(serde_json::to_string)
        .collect::<Result<Vec<_>, _>>()
        .map(|lines| lines.join("\n") + "\n")
}
