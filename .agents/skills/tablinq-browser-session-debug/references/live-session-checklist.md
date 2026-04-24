# Live Session Checklist

Use this checklist before any action that touches a signed-in browser session.

## Before Sending Input

- Confirm a real browser session is required.
- Confirm the page is idle or safe to inspect.
- Read the latest user message.
- Read the latest assistant response start/end.
- Check composer text is empty or intentionally staged.
- Identify current topic and likely next natural follow-up.
- List forbidden repeats.
- Decide whether upload is needed.

## Before Uploading a File

- Generate a unique filename.
- Generate unique content.
- Include a unique id inside the file.
- Ensure content is task-relevant.
- Ensure no secrets, cookies, tokens, private browser data, or credentials are included.
- Record filename and unique id.

## During Observation

- Capture stream deltas where possible.
- Capture final Tablinq CLI/API text.
- Capture browser visible answer text.
- For DOM bugs, capture compact selector diagnostics.
- Do not keep interacting while the model is still responding unless testing interruption.

## After Observation

- Verify no repeated prompt was sent.
- Verify no duplicate upload was used.
- Verify output matches the browser answer.
- Verify ignored ChatGPT UI artifacts are absent from CLI/API.
- Stop after enough evidence is collected.
