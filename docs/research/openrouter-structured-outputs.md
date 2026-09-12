# OpenRouter Structured Outputs (response_format: json_schema) — Primary-Source Research

## Summary

OpenRouter's structured-outputs feature uses the request shape
`response_format: { type: "json_schema", json_schema: { name, strict, schema } }`,
which is the same shape OpenAI itself uses (OpenRouter's docs don't say this in so
many words, but they link straight to OpenAI's own structured-outputs guide as "the"
reference for provider-side behavior). OpenRouter's own docs do **not** provide an
explicit example or statement about `enum` fields specifically — they only say the
feature enforces "specific JSON Schema validation," and separately warn that a
provider's *strict* mode "may restrict which JSON Schema features you can use."
Support is **not universal**: it is determined per model *and per underlying
provider/endpoint*, changes over time, and must be checked via the models API's
`supported_parameters` field (value `structured_outputs`) or filtered via
`?supported_parameters=structured_outputs`. If you don't pin your request to only
structured-output-capable providers, OpenRouter's router may silently route to a
provider that ignores the schema (per the provider-routing docs, `response_format`
is only a "soft preference" by default) — but OpenRouter's structured-outputs page
separately states that the request "will fail with an error indicating lack of
support" if the model itself lacks support. The docs' own recommended way to get a
hard guarantee is to set `require_parameters: true` in provider preferences plus
`response_format`/`type: json_schema` in the request, so unsupported endpoints are
excluded from routing entirely rather than relying on the error path.

## 1. Exact request-body shape for `response_format` / JSON Schema

Source: [Structured Outputs](https://openrouter.ai/docs/guides/features/structured-outputs)

The documented example (a "weather" schema) is:

```json
{
  "response_format": {
    "type": "json_schema",
    "json_schema": {
      "name": "weather",
      "strict": true,
      "schema": {
        "type": "object",
        "properties": {
          "location": {
            "type": "string",
            "description": "City or location name"
          },
          "temperature": {
            "type": "number",
            "description": "Temperature in Celsius"
          },
          "conditions": {
            "type": "string",
            "description": "Weather conditions description"
          }
        },
        "required": ["location", "temperature", "conditions"],
        "additionalProperties": false
      }
    }
  }
}
```

This matches your assumed shape exactly: top-level `type: "json_schema"`, sibling
`json_schema` object with `name`, `strict`, and `schema` keys, and the schema itself
is a normal JSON Schema object (`type: "object"`, `properties`, `required`,
`additionalProperties`).

Confirmed separately via [API Reference: Parameters](https://openrouter.ai/docs/api-reference/parameters):
`response_format` is documented as an optional map/object parameter. The reference
also documents a distinct boolean model-capability field, `structured_outputs`
("if the model can return structured outputs using response_format json_schema"),
which is separate from the `response_format` request parameter itself — see §3.

The docs also show the identical schema reused unchanged across the TypeScript SDK,
raw `fetch`, and Python examples on the same page — i.e., OpenRouter doesn't alter
the schema shape per SDK/language, it's the same JSON body in all cases.

## 2. Does OpenRouter honor `enum` constraints on a string field?

**Not explicitly confirmed by OpenRouter's own docs.** The
[Structured Outputs](https://openrouter.ai/docs/guides/features/structured-outputs)
page never uses the word "enum" anywhere, and its only worked example (`weather`)
has no enum-constrained field — `conditions` is a plain unconstrained `"type": "string"`,
not `"enum": [...]`.

What the docs *do* say, which bears on this question:

- The feature's stated purpose is to "Enforce specific JSON Schema validation on
  model responses" (Overview section, same URL) — `enum` is a standard JSON Schema
  keyword, so it falls under this general claim, but it is not called out by name.
- Under **Best Practices**: "Use strict mode: Set `strict: true` so that providers
  with a native strict mode enforce your schema exactly. Enforcement varies by
  provider: some guarantee schema-conforming output, while others translate your
  schema into their own structured-output format or treat it as a strong hint, so
  exact compliance is not guaranteed on every endpoint. **Strict modes may also
  restrict which JSON Schema features you can use.** See the provider's
  documentation for details." (same page)
- Under **Model Support**, OpenRouter explicitly disclaims owning the enforcement
  semantics and instead defers to each upstream provider's own docs: "For details
  on each provider's implementation, see their documentation, for example:
  [OpenAI](https://platform.openai.com/docs/guides/structured-outputs),
  [Google Gemini](https://ai.google.dev/gemini-api/docs/structured-output),
  [Anthropic](https://docs.claude.com/en/docs/build-with-claude/structured-outputs),
  [Fireworks](https://docs.fireworks.ai/structured-responses/structured-response-formatting#structured-response-modes)."
  (same page)

**Bottom line:** OpenRouter's own primary docs do not explicitly confirm `enum` is
honored, and explicitly warn that "which JSON Schema features you can use" is
provider-dependent under strict mode. This is an OpenRouter-docs gap, not a
confirmation either way — if `enum` enforcement is load-bearing for the app (as it
is here, for constraining `endpoint` to a live enum of GET paths), this should be
verified empirically against whatever model/provider is chosen at deploy time,
per OpenRouter's own suggestion to consult the specific provider's docs (e.g.
OpenAI's structured-outputs guide, which does document `enum` support under strict
mode).

## 3. Is structured-output support universal, or model/provider-dependent?

**Model/provider (technically per-*endpoint*) dependent, not universal.**
Source: [Structured Outputs — Model Support](https://openrouter.ai/docs/guides/features/structured-outputs)

Exact quote: "Structured outputs are supported by select models. You can find a
list of models that support structured outputs on the [models
page](https://openrouter.ai/models?order=newest&supported_parameters=structured_outputs).
**Support is determined per endpoint, not just per model**: the same model may be
served by multiple providers, and only some of those providers may support
structured outputs. Endpoint support can also change over time. To see which
providers support structured outputs for a specific model, check the
`structured_outputs` parameter in the Providers section of the model's page."

### How to check support programmatically

Per [Overview: Models](https://openrouter.ai/docs/guides/overview/models):

- Every model object returned by the models API has a `supported_parameters:
  string[]` field listing which OpenAI-compatible request parameters that model
  (endpoint) accepts, including values like `tools`, `tool_choice`, `max_tokens`,
  `temperature`, `top_p`, `reasoning`, `structured_outputs`, `response_format`,
  `stop`, `frequency_penalty`, `presence_penalty`, `seed`, etc. `structured_outputs`
  is a distinct capability flag from `response_format` (a model can in principle
  support `response_format` JSON-mode without supporting full JSON-Schema
  `structured_outputs`).
- The `/api/v1/models` endpoint accepts a `supported_parameters` query filter, e.g.
  `curl "https://openrouter.ai/api/v1/models?supported_parameters=tools"`, and by
  extension `?supported_parameters=structured_outputs` to list only models/endpoints
  that support this feature (this exact filter URL is also linked directly from the
  structured-outputs doc page itself, see above).

### Documented behavior when the configured model does NOT support it

Two distinct, and only partially reconciled, statements exist in OpenRouter's own
docs:

1. **Structured Outputs page → Error Handling** (hard failure):
   "When using structured outputs, you may encounter these scenarios: 1. **Model
   doesn't support structured outputs**: The request will fail with an error
   indicating lack of support. 2. **Invalid schema**: The model will return an
   error if your JSON Schema is invalid."
   — [Structured Outputs](https://openrouter.ai/docs/guides/features/structured-outputs)

2. **Provider Routing page → default soft-preference behavior** (silent
   degradation possible unless pinned):
   "a small set of parameters is used as a soft preference when choosing between
   providers of the same model: `tools`, `response_format` (including structured
   outputs), and `verbosity`." With `require_parameters` left at its default
   (`false`), OpenRouter will still route to a provider for the chosen model even
   if that provider doesn't support `response_format`/structured outputs — it's
   only a *preference*, not a hard filter, when choosing between providers of the
   same model. Setting `require_parameters: true` turns it into a hard filter:
   "Only use providers that support all parameters in your request." Documented
   example:
   ```json
   {
     "provider": { "require_parameters": true },
     "response_format": { "type": "json_object" }
   }
   ```
   — [Provider Routing](https://openrouter.ai/docs/features/provider-routing)

**Practical reading:** the "will fail with an error" statement is the documented
behavior for a request routed to an endpoint/provider that plainly cannot do
structured outputs at all; the provider-routing soft-preference language describes
what happens *among multiple providers behind the same model slug* when some
support it and some don't and you haven't pinned `require_parameters: true` — in
that case OpenRouter prefers a capable provider but does not guarantee one is
picked. OpenRouter's own **recommended defensive pattern**, stated directly on the
structured-outputs page, is:
"To ensure your request is only routed to endpoints that support structured
outputs: 1. Check the model's supported parameters on the [models
page](https://openrouter.ai/models) 2. Set `require_parameters: true` in your
provider preferences (see [Provider
Routing](/docs/guides/routing/provider-selection)) 3. Include `response_format` and
set `type: json_schema` in the required parameters."
— [Structured Outputs](https://openrouter.ai/docs/guides/features/structured-outputs)

This is a general, model-agnostic mechanism (checking `supported_parameters` +
`require_parameters: true`), so it applies regardless of which specific model is
chosen at deploy time.

## 4. Gotchas for a hand-rolled Rust `ureq`/`serde_json` request body

Source for all items below unless noted: [Structured
Outputs](https://openrouter.ai/docs/guides/features/structured-outputs).

- **`strict: true`** — not stated as strictly *required*, but is the documented
  Best Practice: "Use strict mode: Set `strict: true` so that providers with a
  native strict mode enforce your schema exactly." Caveat: "Enforcement varies by
  provider: some guarantee schema-conforming output, while others translate your
  schema into their own structured-output format or treat it as a strong hint, so
  exact compliance is not guaranteed on every endpoint. Strict modes may also
  restrict which JSON Schema features you can use." So `strict: true` is the right
  default to set, but it is not a guarantee of schema conformance across every
  provider OpenRouter might route to — this reinforces the need to pin a
  known-good provider (§3) if enum-constraint enforcement is load-bearing.

- **`additionalProperties: false`** — appears in OpenRouter's own worked example
  schema (quoted in §1) but the docs do **not** state in prose that it is
  *required*. No explicit "you must set this" sentence exists on the page; it's
  present only as part of the example, mirroring the equivalent OpenAI
  requirement (which OpenRouter's docs link out to rather than restate). Treat as
  documented-by-example, not documented-by-mandate.

- **Schema size limits / max nesting depth / `$ref` support / other structural
  requirements** — **not mentioned anywhere on OpenRouter's structured-outputs
  page.** No "Limitations," "Caveats," or "Notes" section exists on that page at
  all. This is a real documentation gap: OpenRouter does not publish its own
  limits and instead implicitly defers to whatever the routed provider enforces
  (see the provider links in §2/§3). Do not assume OpenAI's specific limits
  (e.g. `$ref`/depth restrictions) automatically apply — they may, since OpenAI is
  one of the providers OpenRouter can route to, but OpenRouter itself makes no
  documented promise here.

- **`name` field** — present in every example (`"name": "weather"`) as a sibling of
  `strict` and `schema` inside `json_schema`, but the docs don't specify format/length
  constraints for it beyond the example usage.

- **Root type must be `object`** — not explicitly stated as a rule, but every
  example on the page uses `"type": "object"` at the schema root; no
  counter-example (e.g. root array or scalar) is shown or discussed.

- **Provider-routing implications / need to pin a provider** — this is the single
  most load-bearing gotcha found. Structured-outputs support is per-*endpoint*
  (model × provider), can change over time, and — absent
  `require_parameters: true` — is only a soft routing preference, not a guarantee,
  when multiple providers back the same model. The docs' own three-step mitigation
  (check `supported_parameters`, set `require_parameters: true`, include
  `response_format`/`type: json_schema` as a required parameter) should be treated
  as mandatory for this app's Routing Call, not optional hardening — otherwise a
  request could silently be routed to a provider that ignores the `endpoint` enum
  constraint entirely, defeating the whole purpose of the Plan Schema.

- **Streaming compatibility** — not directly relevant to this app's synchronous
  `ureq` usage, but noted: "Structured outputs are also supported with streaming
  responses... To enable streaming with structured outputs, simply add
  `stream: true` to your request." (same page) — i.e., streaming and structured
  outputs are not mutually exclusive on OpenRouter, in case that becomes relevant
  later.

- **No explicit OpenAI-parity statement** — OpenRouter's docs never state in so
  many words "this matches OpenAI's `response_format` spec." They instead link out
  to OpenAI's (and Gemini's, Anthropic's, Fireworks') own structured-outputs docs
  as the source of truth for *provider-specific* enforcement behavior, while
  defining the outer `response_format`/`json_schema` request envelope themselves.
  Practically, the request shape is OpenAI-envelope-compatible (per the worked
  example), but per-provider enforcement fidelity (e.g. enum handling) is
  explicitly not something OpenRouter itself warrants — see §2.

## Sources

- https://openrouter.ai/docs/guides/features/structured-outputs — primary source for request shape, Model Support, Best Practices, Error Handling, Streaming sections (fetched multiple times for different sections/verbatim quotes)
- https://openrouter.ai/docs/api-reference/parameters — `response_format` and `structured_outputs` parameter reference
- https://openrouter.ai/docs/guides/overview/models — `supported_parameters` field definition and `?supported_parameters=` filtering
- https://openrouter.ai/docs/features/provider-routing — `require_parameters`, soft-preference behavior for `response_format`/`tools`/`verbosity`
