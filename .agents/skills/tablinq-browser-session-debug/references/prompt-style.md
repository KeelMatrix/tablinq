# Prompt Style for Live ChatGPT Debug Sessions

## Goal

Generate prompts that resemble plausible real user curiosity while still being useful for Tablinq debugging. The prompt should fit the current conversation instead of looking like a synthetic benchmark.

## Prompt Rules

Prefer:

- first-person or direct phrasing when natural
- short follow-up questions
- small imperfections in rhythm, not typos for their own sake
- domain-relevant curiosity
- concrete constraints
- one question at a time

Avoid:

- “Please provide a comprehensive, detailed...”
- “As an AI language model...”
- “Act as...” unless the test explicitly needs role prompting
- unnatural keyword stuffing
- repeating the current topic from scratch
- overly clean numbered requirements unless testing formatting
- prompts that reveal automation/debug instructions

## Duplicate Check

Before sending a prompt, compare it against:

- the latest user message
- the last 3 to 5 user messages if visible
- any prompts already sent by this debugging task
- pending composer text

Treat these as duplicates and do not send:

- same words with punctuation changes
- same request with only “please” added
- same request with synonyms but same intent
- asking again because the previous response was slow
- a prompt copied from the bug report if it already exists in the current chat

## Continuation Examples

Given previous topic: procedural AI game worlds.

Good:

```text
what would you cut first if this had to ship in 6 months?
```

```text
ok, but what does the smallest convincing prototype look like?
```

```text
which part would fool the team into thinking it's working when it's not?
```

Bad:

```text
what would it take to create a complex, procedurally generated game world with modern AI?
```

```text
Please provide a detailed practical production plan for world generation layers, AI components, runtime architecture, team/cost drivers, and risks.
```

## Research Prompts

When research is needed, ask naturally and with scope:

Good:

```text
can you check current docs and tell me what changed around this api lately?
```

```text
what are people actually using for this now, not the marketing version?
```

Bad:

```text
Research the latest information on X and provide citations in a structured report.
```

## Upload Follow-ups

When uploading a file, ask a question that refers to the actual file contents:

Good:

```text
can you look at this tiny repro and tell me where the stream goes stale?
```

```text
this is a clipped log from the freeze. what stands out?
```

Bad:

```text
Analyze the attached file.
```
