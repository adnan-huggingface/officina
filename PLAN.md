# Assist, phase 1: the `assist` crate

The first of six phases that give Scriva and Calx an assistant a person talks
to while editing. The specification, with every decision and why it was taken,
is kept in the story workspace (`bugs/assist-spec.md`); this plan is its first
phase. **This file is immutable while the work runs.** Nothing that does the
work may edit it: not to reword an item, not to remove one, and above all not
to mark one done. The harness plan that stood here before is in the history,
and every item of it proved itself before it was replaced.

Each item carries a `verify:` command and is finished exactly when that command
exits zero. `python .claude/hooks/gate.py` runs them all. A `cargo test` filter
that matches no test exits zero, so every test item goes through
`.claude/hooks/proved.py`, which insists a test ran.

## Why

An assistant is two halves: the helper that answers, and the editor that acts
on what it says. This phase builds the first half with no window and no
document in sight, so that every way a helper can answer — text, a request to
run a tool, a refusal, a rate limit, a dropped connection, silence — is pinned
down by a test before a pane ever shows one.

Three helpers speak over HTTP and one is a script. Claude is reached through
Anthropic's Messages API with the user's own key or the login the `ant` command
keeps; Ollama and any other service are reached through the chat-completions
shape they share; the script plays canned turns and is the only helper a test
may use. The helper that runs on the computer itself is phase 5, and so is the
measurement of its speed: the user decided on 2026-09-17 that it ships in pure
Rust whatever that measurement says, so nothing in this phase waits on it.

The rules this phase makes true for the rest:

- **A test never reaches the network.** A headless process refuses every
  helper but the script, refuses to look for `ant` or Ollama, and counts each
  refusal, as it already refuses file choosers.
- **Nothing is bundled.** HTTPS trusts the certificates the operating system
  trusts; no certificate store is compiled in.
- **Every failure is a sentence.** A request ends in a finished answer or in one
  sentence a person can act on, and never in a wait with no end.
- **The helper's choice is made once for both applications**, in
  `~/.config/officina/assist.toml`, which only its owner can read because a key
  may be in it.

Nothing here touches a reader or a writer: `cargo xtask fidelity` stays at zero
failures.

---

### A1 — a conversation, a scripted helper, and the loop between them

A helper streams events: text as it arrives, the tools it wants run, and one
ending. A session hands each tool call to the host, sends every result back in
one turn, and goes on until the helper ends its turn. A request that fails
leaves the conversation as it was before the request.

    verify: python .claude/hooks/proved.py -p assist a_scripted_provider_plays_its_turns_as_the_events_a_real_one_streams
    verify: python .claude/hooks/proved.py -p assist a_session_hands_each_tool_call_to_the_host_and_sends_every_result_back_in_one_turn
    verify: python .claude/hooks/proved.py -p assist a_request_that_fails_leaves_the_conversation_as_it_was_before_it

### A2 — Claude, over a socket

A stream in the Messages API's shape, served by a listener in the test, becomes
text, tool calls and an ending. The request names the model, marks every tool
strict, caches the system prompt and the conversation, and asks for the
server's fallback on a model that has one. Thinking goes back to Claude exactly
as it came, and what a declined model wrote before a fallback took over is not
sent back as if it stood. A login from `ant` is sent as a bearer token.

    verify: python .claude/hooks/proved.py -p assist an_anthropic_stream_over_a_socket_becomes_text_tool_calls_and_a_stop
    verify: python .claude/hooks/proved.py -p assist the_request_to_anthropic_names_the_model_the_strict_tools_the_cache_and_the_fallback
    verify: python .claude/hooks/proved.py -p assist thinking_goes_back_as_it_came_and_what_a_declined_model_began_does_not
    verify: python .claude/hooks/proved.py -p assist a_login_from_ant_is_sent_as_a_bearer_token_and_a_key_as_a_key

### A3 — another service, over a socket

A chat-completions stream becomes the same events as Claude's, tool calls
assembled from their fragments; a tool's result goes back as a tool message.

    verify: python .claude/hooks/proved.py -p assist a_compatible_stream_over_a_socket_becomes_the_same_events
    verify: python .claude/hooks/proved.py -p assist a_tool_result_goes_to_a_compatible_service_as_a_tool_message

### A4 — failures, and Stop

A refusal, a rate limit and a connection dropped mid-answer each end the
request with a sentence, promptly. A service that is not there is a sentence
naming it. Stop ends a request between two chunks and lets the connection go.

    verify: python .claude/hooks/proved.py -p assist a_refusal_a_rate_limit_and_a_dropped_connection_are_each_a_sentence_not_a_hang
    verify: python .claude/hooks/proved.py -p assist a_service_that_is_not_there_is_a_sentence_naming_it
    verify: python .claude/hooks/proved.py -p assist stop_ends_a_request_between_two_chunks

### A5 — what the computer has, and what the user chose

The first-run card's rows come from what was found, in the card's order: a
Claude login first, then Ollama, then the helper on this computer, which is
first when nothing was found; Claude with a key and another service are always
offered. The settings file keeps a key where only its owner can read it, and a
file that cannot be read is the defaults rather than an error. Each Claude model
is offered with what a paragraph costs on it.

    verify: python .claude/hooks/proved.py -p assist the_ladder_offers_what_the_machine_has_in_the_order_the_card_shows
    verify: python .claude/hooks/proved.py -p assist a_key_is_written_to_a_file_only_its_owner_can_read
    verify: python .claude/hooks/proved.py -p assist settings_that_cannot_be_read_are_the_defaults
    verify: python .claude/hooks/proved.py -p assist a_paragraph_costs_about_two_cents_on_opus_and_under_one_on_sonnet

### A6 — no helper from a test, and a place for what is downloaded

A headless process refuses every helper but the scripted one, and counts the
refusals, without opening a connection. The cache directory follows the
configuration directory's rule on every platform, and both move under the
temporary directory in a test. Assist's settings are one file for both
applications.

    verify: python .claude/hooks/proved.py -p ui-kit a_headless_process_refuses_every_helper_but_the_scripted_one_and_counts_it
    verify: python .claude/hooks/proved.py -p ui-kit the_cache_directory_follows_the_config_directorys_rule_on_every_platform
    verify: python .claude/hooks/proved.py -p ui-kit assists_settings_are_one_file_for_both_applications

### A7 — nothing bundled

No root certificate store is compiled into either application on Linux or
Windows: HTTPS trusts what the operating system trusts, through the platform
verifier, which must be in the tree. (Weak in the way a dependency check is: it
proves the crate is absent, not how the verifier is configured; `http.rs` says
which verifier is used and why.)

Corrected once, on 2026-09-17, before any item had been run: the first version
of this line read `cargo tree`'s exit status, which is zero when a crate is in
the lockfile but not in the graph asked about ("nothing to print"), so it could
never pass. It now reads what the tree prints, and fails unless the verifier is
printed too, so that a `cargo tree` that printed nothing at all is not a pass.

    verify: python -c "import subprocess,sys; tree=lambda t,c: subprocess.run(['cargo','tree','-e','normal','--target',t,'-p','assist','-p','calx','-p','scriva','-i',c],capture_output=True,text=True).stdout; targets=('x86_64-unknown-linux-gnu','x86_64-pc-windows-msvc'); bad=[(t,c) for t in targets for c in ('webpki-roots','webpki-root-certs') if c in tree(t,c)]; verified=all('rustls-platform-verifier' in tree(t,'rustls-platform-verifier') for t in targets); print(bad or ('no certificate store compiled in' if verified else 'the platform verifier is not in the tree')); sys.exit(0 if verified and not bad else 1)"

## The end

### Z1 — the page did not move, and the files did not change

    verify: cargo xtask compare --check
    verify: cargo xtask fidelity
