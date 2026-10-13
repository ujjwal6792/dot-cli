// Package setup owns first-run effects; configuration policy remains in native.
package setup

import (
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"runtime"
	"strings"

	"github.com/ujjwal6792/dots-cli/internal/native"
	"github.com/ujjwal6792/dots-cli/internal/tui"
)

type Options struct {
	Home, Source, State, Config, OS, Git, Chezmoi string
	// User and OSRelease are optional suggestion inputs, not executable adapters.
	User, OSRelease string
	// Manager paths are resolved by #18 or LookPath, never guessed private helpers.
	Brew, Pacman, Sudo string
	Name, Email        string
}
type Connection struct {
	Version int            `json:"version"`
	Source  string         `json:"source"`
	Home    string         `json:"home"`
	Config  string         `json:"config"`
	State   string         `json:"state"`
	Machine native.Machine `json:"machine"`
}
type Engine struct {
	*tui.NativeEngine
	options Options
}

// canonicalPath recognizes only OS-owned aliases on Darwin, not user symlinks.
func canonicalPath(p string) (string, error) {
	if !filepath.IsAbs(p) || filepath.Clean(p) != p {
		return "", fmt.Errorf("explicit clean absolute paths required")
	}
	if runtime.GOOS == "darwin" {
		for _, prefix := range []string{"/var", "/tmp", "/etc"} {
			if p == prefix || strings.HasPrefix(p, prefix+"/") {
				p = "/private" + p
				break
			}
		}
	}
	if p == "/" {
		return "", fmt.Errorf("filesystem root is not a setup path")
	}
	if _, err := native.BoundedPath(filepath.Dir(p), filepath.Base(p)); err != nil {
		return "", err
	}
	return p, nil
}
func New(o Options) (*Engine, error) {
	if o.Home == "" {
		o.Home = os.Getenv("HOME")
	}
	var err error
	if o.Home, err = canonicalPath(o.Home); err != nil {
		return nil, err
	}
	if o.State == "" {
		o.State = os.Getenv("DOT_STATE_DIR")
	}
	if o.State == "" {
		if x := os.Getenv("XDG_STATE_HOME"); x != "" {
			base, err := canonicalPath(x)
			if err != nil {
				return nil, err
			}
			o.State = filepath.Join(base, "dot")
		} else {
			o.State = filepath.Join(o.Home, ".local/state/dot")
		}
	}
	if o.Source == "" {
		o.Source = os.Getenv("DOT_SOURCE")
	}
	if o.Config == "" {
		o.Config = os.Getenv("DOT_CONFIG")
	}
	if o.State, err = canonicalPath(o.State); err != nil {
		return nil, err
	}
	if _, err := native.BoundedPath(o.State, "connection.json"); err != nil {
		return nil, err
	}
	var c Connection
	b, err := os.ReadFile(filepath.Join(o.State, "connection.json"))
	if err == nil {
		if err := native.Decode(b, &c); err != nil {
			return nil, fmt.Errorf("malformed connection record: %w", err)
		}
		if c.Version != 1 || c.State != o.State || c.Home != o.Home {
			return nil, fmt.Errorf("connection path/version conflict")
		}
		for _, p := range []string{c.Source, c.Config} {
			if _, err := canonicalPath(p); err != nil {
				return nil, fmt.Errorf("connection path invalid: %w", err)
			}
		}
		if err := native.ValidMachine(c.Machine); err != nil {
			return nil, err
		}
		if o.Source == "" {
			o.Source = c.Source
		}
		if o.Config == "" {
			o.Config = c.Config
		}
	} else if !os.IsNotExist(err) {
		return nil, err
	}
	if o.Source == "" {
		o.Source = filepath.Join(o.Home, ".local/share/dot-config")
	}
	if o.Config == "" {
		if x := os.Getenv("XDG_CONFIG_HOME"); x != "" {
			base, err := canonicalPath(x)
			if err != nil {
				return nil, err
			}
			o.Config = filepath.Join(base, "chezmoi/chezmoi.toml")
		} else {
			o.Config = filepath.Join(o.Home, ".config/chezmoi/chezmoi.toml")
		}
	}
	if o.Source, err = canonicalPath(o.Source); err != nil {
		return nil, err
	}
	if o.Config, err = canonicalPath(o.Config); err != nil {
		return nil, err
	}
	if filepath.Ext(o.Config) != ".toml" {
		return nil, fmt.Errorf("tracker config must be an explicit TOML path")
	}
	if c.Version != 0 && (c.Source != o.Source || c.Config != o.Config) {
		return nil, fmt.Errorf("connection conflicts with explicit source/config; no paths switched")
	}
	if o.OS == "" {
		o.OS = runtime.GOOS
	}
	if o.Git == "" {
		o.Git = "git"
	}
	if overlaps(o.Config, o.Source) || overlaps(o.Config, o.State) || o.State == o.Home || overlaps(o.Home, o.State) {
		return nil, fmt.Errorf("config/source/state boundaries overlap")
	}
	p, err := native.New(native.Config{Source: o.Source, Home: o.Home, State: o.State, TrackerConfig: o.Config, OS: o.OS, Git: o.Git, Chezmoi: o.Chezmoi})
	if err != nil {
		return nil, err
	}
	e := &Engine{NativeEngine: tui.NewNativeEngine(p), options: o}
	a, adoptionErr := p.LoadAdoption()
	if adoptionErr != nil && !errors.Is(adoptionErr, os.ErrNotExist) {
		return nil, adoptionErr
	}
	if c.Version != 0 {
		if adoptionErr != nil {
			return nil, adoptionErr
		}
		if a.Machine != c.Machine {
			return nil, fmt.Errorf("connection machine conflicts with adoption")
		}
	}
	return e, nil
}
func overlaps(p, root string) bool {
	r, e := filepath.Rel(root, p)
	return e == nil && r != ".." && !strings.HasPrefix(r, "../")
}
func (e *Engine) Options() Options { return e.options }
func encode(v any) []byte          { b, _ := json.MarshalIndent(v, "", "  "); return append(b, '\n') }
func exclusive(path string, b []byte) error {
	if _, err := canonicalPath(path); err != nil {
		return err
	}
	if err := os.MkdirAll(filepath.Dir(path), 0700); err != nil {
		return err
	}
	if _, err := canonicalPath(path); err != nil {
		return err
	}
	f, err := os.CreateTemp(filepath.Dir(path), ".dot-setup-")
	if err != nil {
		return err
	}
	defer os.Remove(f.Name())
	if err = f.Chmod(0600); err == nil {
		_, err = f.Write(b)
	}
	if err == nil {
		err = f.Sync()
	}
	ce := f.Close()
	if err != nil {
		return err
	}
	if ce != nil {
		return ce
	}
	if err := os.Link(f.Name(), path); err != nil {
		return err
	}
	d, err := os.Open(filepath.Dir(path))
	if err != nil {
		return err
	}
	defer d.Close()
	return d.Sync()
}
