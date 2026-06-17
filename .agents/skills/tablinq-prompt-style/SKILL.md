---
name: tablinq-prompt-style
description: Shape realistic live ChatGPT prompts for Tablinq debugging so prompts fit the active conversation, avoid duplicates, and exercise the UI without sounding synthetic.
---

# Tablinq Prompt Style

Use this skill when Tablinq work needs a prompt that may be sent into a real signed-in ChatGPT session. This skill only covers prompt wording and duplicate avoidance. Pair it with `tablinq-browser-session-debug` when the task also needs live-session inspection, uploads, or DOM validation.

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

- "Please provide a comprehensive, detailed..."
- "As an AI language model..."
- "Act as..." unless the test explicitly needs role prompting
- unnatural keyword stuffing
- repeating the current topic from scratch
- overly clean numbered requirements unless testing formatting
- prompts that reveal automation/debug instructions
- prompts that ask for a fixed sentinel phrase or exact test response

## Duplicate Check

Before sending a prompt, compare it against:

- the latest user message
- the last 3 to 5 user messages if visible
- any prompts already sent by this debugging task
- pending composer text

Treat these as duplicates and do not send:

- same words with punctuation changes
- same request with only "please" added
- same request with synonyms but same intent
- asking again because the previous response was slow
- a prompt copied from the bug report if it already exists in the current chat

## Content-Irrelevant Validation Prompts

When the prompt content itself is not under test and the live prompt is only being sent to validate routing, model selection, UI selection, transport, or gateway/CLI wiring:

- choose a normal standalone user question from an everyday topic unrelated to Tablinq or debugging
- do not mention Tablinq, local automation, UI drift, smoke checks, routing, the gateway, the CLI, test state, or validation
- do not ask for an exact phrase, sentinel text, or "reply with X"
- vary the topic across multiple prompts instead of reusing the same template with different expected words
- keep prompts short enough for fast validation, but make them sound like real questions a person might ask

Good:

```text
what's a good way to keep basil alive on a sunny windowsill?
```

```text
how should I think about choosing between a train and a short flight?
```

```text
what's one practical way to make a small kitchen feel less cramped?
```

Bad:

```text
For a local Tablinq UI drift check, reply in one short sentence: medium route ok.
```

```text
Say exactly "instant route ok" so I can verify the gateway path.
```

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
