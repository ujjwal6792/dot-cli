# One native application and one interactive model

Spec #10 replaces fragmented tty, gum, curses and subprocess browser/engine flows
with one Go executable. Bubble Tea v2, Bubbles, Lipgloss and embedded Huh forms
serve setup and returning-user operations; scriptable verbs call the same engines.
This trades compatibility with private script runners for a single safety policy,
terminal lifecycle and independently distributable artifact. #7's launcher wording,
#8's owner-specific source and #9's hidden help are subsumed by the generic app/help.
