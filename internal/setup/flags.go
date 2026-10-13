package setup

import "flag"

// BindFlags registers the common setup inputs, without routing commands or
// authorizing effects. #18 registers action flags (--check/--url/etc.) itself.
func (o *Options) BindFlags(fs *flag.FlagSet) {
	fs.StringVar(&o.Home, "home", o.Home, "explicit target HOME (default HOME environment)")
	fs.StringVar(&o.Source, "source", o.Source, "explicit private checkout root")
	fs.StringVar(&o.Config, "config", o.Config, "explicit local tracker TOML file")
	fs.StringVar(&o.State, "state-dir", o.State, "isolated machine state and connection directory")
	fs.StringVar(&o.Git, "git", o.Git, "system Git executable (no private adapter fallback)")
	fs.StringVar(&o.Chezmoi, "chezmoi", o.Chezmoi, "explicit resolved chezmoi executable; existing installations are not overwritten")
	fs.StringVar(&o.Name, "name", o.Name, "tracker-only name fallback when Git user.name is unset")
	fs.StringVar(&o.Email, "email", o.Email, "tracker-only email fallback when Git user.email is unset")
}
