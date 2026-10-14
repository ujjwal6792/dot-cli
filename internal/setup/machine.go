package setup

import (
	"context"
	"crypto/rand"
	"encoding/json"
	"errors"
	"fmt"
	"math/big"
	"os"
	"os/user"
	"path/filepath"
	"strings"

	"github.com/ujjwal6792/dots-cli/internal/native"
	"github.com/ujjwal6792/dots-cli/internal/tui"
)

func token(s string) string {
	var b strings.Builder
	for _, r := range strings.ToLower(s) {
		if r >= 'a' && r <= 'z' || r >= '0' && r <= '9' || r == '_' {
			b.WriteRune(r)
		} else {
			b.WriteByte('_')
		}
	}
	return strings.Trim(b.String(), "_")
}
func (e *Engine) catalog(ctx context.Context) (map[string]native.MachineInfo, error) {
	c, err := e.Policy.CheckContract(ctx)
	if err != nil {
		return nil, err
	}
	p, err := native.BoundedPath(e.options.Source, c.Manifest)
	if err != nil {
		return nil, err
	}
	b, err := os.ReadFile(p)
	if err != nil {
		return nil, err
	}
	var m native.Manifest
	if err := native.Decode(b, &m); err != nil {
		return nil, err
	}
	return m.Machines, nil
}
func (e *Engine) SuggestMachine(ctx context.Context) (native.Machine, error) {
	a, err := e.Policy.LoadAdoption()
	if err == nil {
		return a.Machine, nil
	}
	if !errors.Is(err, os.ErrNotExist) {
		return native.Machine{}, err
	}
	catalog, err := e.catalog(ctx)
	if err != nil {
		return native.Machine{}, err
	}
	osToken := "macos"
	if e.options.OS == "linux" {
		osToken = "linux"
		path := e.options.OSRelease
		if path == "" {
			path = "/etc/os-release"
		}
		b, err := os.ReadFile(path)
		if err != nil && !os.IsNotExist(err) {
			return native.Machine{}, err
		}
		fields := map[string]string{}
		for _, line := range strings.Split(string(b), "\n") {
			k, v, ok := strings.Cut(line, "=")
			if ok {
				fields[k] = strings.Trim(v, "\"'")
			}
		}
		if t := token(fields["ID"]); t != "" {
			osToken = t
		} else if t := token(fields["ID_LIKE"]); t != "" {
			osToken = t
		}
	}
	name := e.options.User
	if name == "" {
		u, err := user.Current()
		if err != nil {
			return native.Machine{}, fmt.Errorf("cannot suggest username; edit a unique machine id")
		}
		name = u.Username
	}
	name = token(name)
	if name == "" {
		name = "user"
	}
	base := osToken + "_" + name
	if len(base) > 125 {
		base = base[:125]
	}
	const chars = "abcdefghijklmnopqrstuvwxyz0123456789"
	if _, exists := catalog[base]; !exists {
		return native.Machine{ID: base, OS: e.options.OS}, nil
	}
	start, err := rand.Int(rand.Reader, big.NewInt(1296))
	if err != nil {
		return native.Machine{}, err
	}
	for n := 0; n < 1296; n++ {
		value := (int(start.Int64()) + n) % 1296
		id := base + "_" + string([]byte{chars[value/36], chars[value%36]})
		if _, exists := catalog[id]; !exists {
			return native.Machine{ID: id, OS: e.options.OS}, nil
		}
	}
	return native.Machine{}, fmt.Errorf("machine collision suffixes exhausted; edit a unique id")
}
func (e *Engine) SaveMachine(ctx context.Context, id string) error {
	m := native.Machine{ID: strings.ToLower(id), OS: e.options.OS}
	if err := native.ValidMachine(m); err != nil {
		return err
	}
	a, err := e.Policy.LoadAdoption()
	if err == nil {
		if a.Machine != m {
			return fmt.Errorf("existing identity cannot be replaced")
		}
		return nil
	}
	if !errors.Is(err, os.ErrNotExist) {
		return err
	}
	catalog, err := e.catalog(ctx)
	if err != nil {
		return err
	}
	if _, exists := catalog[m.ID]; exists {
		return fmt.Errorf("machine id already exists in local catalog; choose a unique id")
	}
	if _, err := native.BoundedPath(e.options.State, "adoption.json"); err != nil {
		return err
	}
	return e.Policy.SaveIdentity(m)
}
func (e *Engine) InitMachine(ctx context.Context, r map[string]any) (tui.MachineResult, error) {
	var req struct {
		Action string `json:"action"`
		ID     string `json:"id,omitempty"`
	}
	b, _ := json.Marshal(r)
	if err := native.Decode(b, &req); err != nil {
		return tui.MachineResult{}, fmt.Errorf("invalid machine request")
	}
	var m native.Machine
	var err error
	switch req.Action {
	case "suggest":
		m, err = e.SuggestMachine(ctx)
	case "save":
		err = e.SaveMachine(ctx, req.ID)
		if err == nil {
			a, er := e.Policy.LoadAdoption()
			m, err = a.Machine, er
		}
	default:
		err = fmt.Errorf("unsupported machine action")
	}
	return tui.MachineResult{Machine: tui.Machine{ID: m.ID, OS: m.OS}}, err
}

// ReadConnection is strict and isolated to the configured state directory.
func (e *Engine) ReadConnection() (Connection, error) {
	var c Connection
	p, err := native.BoundedPath(e.options.State, "connection.json")
	if err != nil {
		return c, err
	}
	b, err := os.ReadFile(p)
	if err != nil {
		return c, err
	}
	if err := native.Decode(b, &c); err != nil {
		return c, fmt.Errorf("malformed connection record")
	}
	a, err := e.Policy.LoadAdoption()
	if err != nil {
		return c, err
	}
	if c != e.connection(a.Machine) {
		return c, fmt.Errorf("connection identity/path conflict")
	}
	return c, nil
}
func (e *Engine) connection(m native.Machine) Connection {
	return Connection{Version: 1, Source: e.options.Source, Home: e.options.Home, Config: e.options.Config, State: e.options.State, Machine: m}
}
func (e *Engine) connect(m native.Machine) error {
	c, err := e.ReadConnection()
	if err == nil {
		if c == e.connection(m) {
			return nil
		}
		return fmt.Errorf("connection conflicts")
	}
	if !errors.Is(err, os.ErrNotExist) {
		return err
	}
	return exclusive(filepath.Join(e.options.State, "connection.json"), encode(e.connection(m)))
}
