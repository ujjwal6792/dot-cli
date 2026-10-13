package native

import (
	"bytes"
	"context"
	"encoding/json"
	"fmt"
	"io"
	"os"
	"path/filepath"
	"regexp"
	"strings"
)

var machineID = regexp.MustCompile(`^[a-z0-9_]{1,128}$`)

func (a *Assignment) UnmarshalJSON(b []byte) error {
	if len(b) > 0 && b[0] == '[' {
		var ids []string
		if err := Decode(b, &ids); err != nil {
			return err
		}
		if len(ids) == 0 {
			ids = []string{"all"}
		}
		a.Include = ids
		a.Exclude = []string{}
		return validateAssignment(a)
	}
	type plain Assignment
	var p plain
	if err := Decode(b, &p); err != nil {
		return err
	}
	*a = Assignment(p)
	return validateAssignment(a)
}
func (x *Entry) UnmarshalJSON(b []byte) error {
	type plain Entry
	var value plain
	if err := Decode(b, &value); err != nil {
		return err
	}
	var fields map[string]json.RawMessage
	if err := json.Unmarshal(b, &fields); err != nil {
		return err
	}
	if raw, ok := fields["machines"]; ok && bytes.Equal(bytes.TrimSpace(raw), []byte("null")) {
		return fmt.Errorf("assignment: null is not an assignment")
	}
	*x = Entry(value)
	return nil
}
func (x *Logical) UnmarshalJSON(b []byte) error {
	type plain Logical
	var value plain
	if err := Decode(b, &value); err != nil {
		return err
	}
	var fields map[string]json.RawMessage
	if err := json.Unmarshal(b, &fields); err != nil {
		return err
	}
	if raw, ok := fields["machines"]; ok && bytes.Equal(bytes.TrimSpace(raw), []byte("null")) {
		return fmt.Errorf("assignment: null is not an assignment")
	}
	*x = Logical(value)
	return nil
}
func (s *Selection) UnmarshalJSON(b []byte) error {
	type plainEntry Entry
	var v struct {
		Path string `json:"path"`
		plainEntry
		Logical      string                    `json:"logical,omitempty"`
		Destinations map[string]string         `json:"destinations,omitempty"`
		Common       map[string]any            `json:"common,omitempty"`
		Machine      map[string]map[string]any `json:"machine,omitempty"`
		OSMetadata   map[string]map[string]any `json:"os_metadata,omitempty"`
	}
	if err := Decode(b, &v); err != nil {
		return err
	}
	var fields map[string]json.RawMessage
	_ = json.Unmarshal(b, &fields)
	if raw, ok := fields["machines"]; ok && bytes.Equal(bytes.TrimSpace(raw), []byte("null")) {
		return fmt.Errorf("assignment: null is not an assignment")
	}
	*s = Selection{Path: v.Path, Entry: Entry(v.plainEntry), Logical: v.Logical, Destinations: v.Destinations, Common: v.Common, Machine: v.Machine, OSMetadata: v.OSMetadata}
	return nil
}
func ValidMachine(m Machine) error {
	if !machineID.MatchString(m.ID) || (m.OS != "linux" && m.OS != "darwin") {
		return fmt.Errorf("invalid machine identity")
	}
	return nil
}

// Decode rejects duplicate/unknown fields at all typed schema layers.
func Decode(data []byte, out any) error {
	d := json.NewDecoder(bytes.NewReader(data))
	if err := unique(d); err != nil {
		return err
	}
	if _, err := d.Token(); err != io.EOF {
		return fmt.Errorf("trailing JSON")
	}
	d = json.NewDecoder(bytes.NewReader(data))
	d.DisallowUnknownFields()
	return d.Decode(out)
}
func unique(d *json.Decoder) error {
	t, err := d.Token()
	if err != nil {
		return err
	}
	switch t {
	case json.Delim('{'):
		seen := map[string]bool{}
		for d.More() {
			k, err := d.Token()
			if err != nil {
				return err
			}
			s := k.(string)
			if seen[s] {
				return fmt.Errorf("duplicate JSON field %q", s)
			}
			seen[s] = true
			if err := unique(d); err != nil {
				return err
			}
		}
		_, err = d.Token()
		return err
	case json.Delim('['):
		for d.More() {
			if err := unique(d); err != nil {
				return err
			}
		}
		_, err = d.Token()
		return err
	}
	return nil
}
func relative(p string) error {
	if p == "" || p == "." || filepath.IsAbs(p) || filepath.Clean(p) != p || strings.ContainsAny(p, "\\*?[]{}\t") {
		return fmt.Errorf("unsafe path %q", p)
	}
	for _, s := range strings.Split(p, "/") {
		if s == ".." || s == ".git" || strings.ContainsAny(s, "\x00\r\n") {
			return fmt.Errorf("unsafe path %q", p)
		}
	}
	return nil
}

// BoundedPath rejects symlinks in the root and every existing path component.
// Missing leaf/parents are allowed, but creation always rechecks this boundary.
func BoundedPath(root, p string) (string, error) {
	if err := relative(p); err != nil {
		return "", err
	}
	if !filepath.IsAbs(root) {
		return "", fmt.Errorf("root must be absolute")
	}
	result := filepath.Join(root, p)
	cur := string(filepath.Separator)
	for _, part := range strings.Split(strings.TrimPrefix(result, string(filepath.Separator)), string(filepath.Separator)) {
		cur = filepath.Join(cur, part)
		i, err := os.Lstat(cur)
		if os.IsNotExist(err) {
			continue
		}
		if err != nil {
			return "", err
		}
		if i.Mode()&os.ModeSymlink != 0 {
			return "", fmt.Errorf("symlink boundary: %s", cur)
		}
	}
	return result, nil
}
func readFile(root, p string) (FileState, error) {
	path, err := BoundedPath(root, p)
	if err != nil {
		return FileState{}, err
	}
	i, err := os.Lstat(path)
	if os.IsNotExist(err) {
		return FileState{}, nil
	}
	if err != nil {
		return FileState{}, err
	}
	if !i.Mode().IsRegular() {
		return FileState{}, fmt.Errorf("not a regular file: %s", p)
	}
	b, err := os.ReadFile(path)
	return FileState{Exists: true, Data: b, Mode: i.Mode().Perm()}, err
}
func readJSON(root, p string, out any) error {
	s, err := readFile(root, p)
	if err != nil {
		return err
	}
	if !s.Exists {
		return os.ErrNotExist
	}
	return Decode(s.Data, out)
}
func replace(root, p string, s FileState) error {
	path, err := BoundedPath(root, p)
	if err != nil {
		return err
	}
	if !s.Exists {
		err = os.Remove(path)
		if os.IsNotExist(err) {
			return nil
		}
		if err != nil {
			return err
		}
		dir, err := os.Open(filepath.Dir(path))
		if err != nil {
			return err
		}
		defer dir.Close()
		return dir.Sync()
	}
	if err := os.MkdirAll(filepath.Dir(path), 0700); err != nil {
		return err
	}
	if _, err := BoundedPath(root, p); err != nil {
		return err
	}
	f, err := os.CreateTemp(filepath.Dir(path), ".dot-write-")
	if err != nil {
		return err
	}
	tmp := f.Name()
	defer os.Remove(tmp)
	if err = f.Chmod(s.Mode.Perm()); err == nil {
		_, err = f.Write(s.Data)
	}
	if err == nil {
		err = f.Sync()
	}
	closeErr := f.Close()
	if err != nil {
		return err
	}
	if closeErr != nil {
		return closeErr
	}
	if err := os.Rename(tmp, path); err != nil {
		return err
	}
	dir, err := os.Open(filepath.Dir(path))
	if err != nil {
		return err
	}
	defer dir.Close()
	return dir.Sync()
}
func jsonState(v any) FileState {
	b, _ := json.MarshalIndent(v, "", "  ")
	return FileState{Exists: true, Data: append(b, '\n'), Mode: 0600}
}
func (e *Engine) LoadAdoption() (Adoption, error) {
	var a Adoption
	if err := readJSON(e.config.State, "adoption.json", &a); err != nil {
		return a, fmt.Errorf("machine-bound adoption required: %w", err)
	}
	if a.Version != 3 || a.Entries == nil || a.Profile == "" {
		return a, fmt.Errorf("malformed adoption v3; refusing legacy downgrade")
	}
	if err := ValidMachine(a.Machine); err != nil {
		return a, err
	}
	if a.Machine.OS != e.config.OS {
		return a, fmt.Errorf("adoption machine OS conflicts with target")
	}
	for id, x := range a.Entries {
		if id == "" {
			return a, fmt.Errorf("empty approved id")
		}
		if err := e.validateEntry(x); err != nil {
			return a, err
		}
	}
	for _, list := range [][]string{a.Profiles, a.Deselected} {
		seen := map[string]bool{}
		for _, id := range list {
			if id == "" || seen[id] {
				return a, fmt.Errorf("malformed duplicate/empty local selection")
			}
			seen[id] = true
		}
	}
	for _, id := range a.Deselected {
		if _, exists := a.Entries[id]; exists {
			return a, fmt.Errorf("conflicting approved/deselected entry")
		}
	}
	destinations := map[string]bool{}
	for _, x := range a.Entries {
		if !applicable(x, a.Machine) || destinations[x.Destination] {
			return a, fmt.Errorf("conflicting machine-bound approval")
		}
		destinations[x.Destination] = true
	}
	for id, c := range a.Customs {
		if id == "" {
			return a, fmt.Errorf("empty custom id")
		}
		if err := validateCustom(c); err != nil {
			return a, err
		}
	}
	return a, nil
}

// SaveIdentity is the setup handoff: only an empty v3 approval may be created;
// an existing identity cannot be replaced, nor can setup authorize entries.
func (e *Engine) SaveIdentity(m Machine) error {
	if err := ValidMachine(m); err != nil {
		return err
	}
	if m.OS != e.config.OS {
		return fmt.Errorf("machine OS conflicts with target")
	}
	_, catalog, err := e.manifest(context.Background())
	if err != nil {
		return err
	}
	if declaration, ok := catalog.Machines[m.ID]; ok && declaration.OS != m.OS {
		return fmt.Errorf("source catalog conflicts with machine OS")
	}
	if _, err := BoundedPath(e.config.State, "adoption.json"); err != nil {
		return err
	}
	if _, err := os.Lstat(filepath.Join(e.config.State, "adoption.json")); err == nil {
		a, err := e.LoadAdoption()
		if err != nil {
			return err
		}
		if a.Machine != m {
			return fmt.Errorf("existing identity cannot be replaced")
		}
		return nil
	} else if !os.IsNotExist(err) {
		return err
	}
	unlock, err := e.lock()
	if err != nil {
		return err
	}
	defer unlock()
	if _, err := os.Lstat(filepath.Join(e.config.State, "recovery.json")); err == nil {
		return fmt.Errorf("identity cannot change during interrupted recovery")
	} else if !os.IsNotExist(err) {
		return err
	}
	if _, err := BoundedPath(e.config.State, "adoption.json"); err != nil {
		return err
	}
	if _, err := os.Lstat(filepath.Join(e.config.State, "adoption.json")); err == nil {
		a, err := e.LoadAdoption()
		if err != nil {
			return err
		}
		if a.Machine != m {
			return fmt.Errorf("existing identity cannot be replaced")
		}
		return nil
	} else if !os.IsNotExist(err) {
		return err
	}
	return replace(e.config.State, "adoption.json", jsonState(Adoption{Version: 3, Profile: "unapproved", Entries: map[string]Entry{}, Machine: m}))
}
