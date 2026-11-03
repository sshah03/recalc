# Running a grid on a local model

recalc runs no model of its own and sends nothing anywhere. The agent in a cell does the reading. With the builtin agents, that's Claude Code or Codex, and the text of every document they answer from goes to Anthropic or OpenAI as part of the prompt, under your account's terms. For documents that can't leave the machine (privileged client files, tax returns, anything under an NDA), put a local model in the cells instead.

## Setup

Install [Ollama](https://ollama.com), pull a model, and name it as an agent in the board's `.recalc/config.toml`:

```toml
[agents]
local = "ollama run --hidethinking gemma4:e2b {task}"
```

Any model you've pulled works in place of `gemma4:e2b`; a larger one answers better and slower. `--hidethinking` keeps a reasoning model's thinking out of the answer.

## The grid

A local model has no tools, so it can't open the file itself. Give it the text: a `$` cell per row reads the document (read-only, so it runs by itself and re-reads when the file changes), and the prompt passes that cell in.

```
        A                         B                                          T
 1  document                  Governing law (state or country only)      (text)
 2  contracts/globex.md       @local("Answer the question from the       $ cat {$A2}
                                      contract text only, with the
                                      answer and nothing else.
                                      Question: {B$1}.
                                      Contract text: {$T2}")
```

Fill B2 across and down, and T2 down, as with any document grid. For PDFs and Word files, replace `cat` with a converter you have (`pdftotext {$A2} -`, `textutil -convert txt -stdout {$A2}` on macOS).

Tried on the legal demo's Globex agreement with `gemma4:e2b`: 4 seconds a cell, answer "the laws of the State of New York". The contract's governing-law section says New York.

## What you give up

- **Citations.** A model without tools can't call `cite_cell`, so answers carry no passage. Add a column asking for the verbatim sentence the answer rests on, and read the two together.
- **Answer types hold, but don't coach.** A strict answer type still refuses a misfit (the run lands red with the answer in its console). A cloud agent told what form to answer in fixes it; a small local model may need a plainer header, like "Yes or No only".
- **Cost and turns.** `ollama run` reports no usage, so the meters stay empty for these cells.
- **Long documents.** The text goes in the prompt, so it has to fit the model's context window; a 128k-context model (`gemma4:e2b-128k`) takes most contracts whole.

A local model with tool use and MCP (for citations and the rest of the board) is the next step. `codex --oss` runs Codex's agent on an Ollama model, but the small model tried here didn't use tools; it needs one trained for tool calling.
