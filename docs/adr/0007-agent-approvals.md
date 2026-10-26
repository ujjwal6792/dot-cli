# Agents act locally; publication and destruction need human approval

Agents may inspect (never secret values), track, stage, assign and commit. When an agent requests push, discard, apply over local changes, history rewrite, branch deletion, machine retire or a remote change, the request becomes a pending item that a human approves in the TUI or an interactive terminal. This replaces v1's plan-ID/approve ceremony, which agents could satisfy themselves. It is a guard against mistakes, not a security boundary against a hostile process running as the same user.
