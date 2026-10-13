package native

import (
	"context"
	"fmt"
	"os"
	"os/exec"
	"strings"
)

func (e *Engine) tempGit(ctx context.Context, s FileState, fn func(string) error) (FileState, error) {
	f, err := os.CreateTemp("", "dot-gitconfig-")
	if err != nil {
		return s, err
	}
	name := f.Name()
	defer os.Remove(name)
	if _, err = f.Write(s.Data); err != nil {
		_ = f.Close()
		return s, err
	}
	if err = f.Close(); err != nil {
		return s, err
	}
	if err = fn(name); err != nil {
		return s, err
	}
	b, err := os.ReadFile(name)
	s.Data = b
	s.Exists = true
	if s.Mode == 0 {
		s.Mode = 0600
	}
	return s, err
}
func (e *Engine) configCommand(ctx context.Context, file string, args ...string) ([]byte, error) {
	cmd := exec.CommandContext(ctx, e.config.Git, append([]string{"config", "--file", file, "--no-includes"}, args...)...)
	b, err := cmd.Output()
	return b, err
}
func (e *Engine) readGitValues(ctx context.Context, root, path string, keys []string) (map[string]string, error) {
	s, err := readFile(root, path)
	if err != nil {
		return nil, err
	}
	if !s.Exists {
		return nil, fmt.Errorf("missing bounded Git source")
	}
	values := map[string]string{}
	_, err = e.tempGit(ctx, s, func(file string) error {
		for _, k := range keys {
			if k != "user.name" && k != "user.email" {
				return fmt.Errorf("unbounded Git key")
			}
			b, err := e.configCommand(ctx, file, "--get-all", k)
			if err != nil {
				return fmt.Errorf("missing or malformed bounded Git value %s", k)
			}
			v := strings.TrimSuffix(string(b), "\n")
			if strings.ContainsAny(v, "\x00\r\n") {
				return fmt.Errorf("ambiguous/multiline Git value")
			}
			values[k] = v
		}
		return nil
	})
	return values, err
}
func (e *Engine) editGit(ctx context.Context, s FileState, values map[string]string) (FileState, error) {
	return e.tempGit(ctx, s, func(file string) error {
		for _, k := range sortedKeys(values) {
			if k != "user.name" && k != "user.email" {
				return fmt.Errorf("unbounded Git key")
			}
			if _, err := e.configCommand(ctx, file, "--replace-all", k, values[k]); err != nil {
				return fmt.Errorf("invalid Git config: %w", err)
			}
		}
		return nil
	})
}
func (e *Engine) gitEntryValues(ctx context.Context, x Entry) (map[string]string, error) {
	if x.GitSource != "" && len(x.GitValues) == 0 {
		source, err := readFile(e.config.Source, x.GitSource)
		if err != nil {
			return nil, err
		}
		if !source.Exists {
			return nil, fmt.Errorf("missing declarative Git source")
		}
		rendered, err := e.renderTemplate(Entry{}, source)
		if err != nil {
			return nil, err
		}
		values := map[string]string{}
		_, err = e.tempGit(ctx, rendered, func(file string) error {
			for _, k := range x.GitKeys {
				b, err := e.configCommand(ctx, file, "--get-all", k)
				if err != nil {
					return fmt.Errorf("missing declarative Git value %s", k)
				}
				value := strings.TrimSuffix(string(b), "\n")
				if strings.ContainsAny(value, "\x00\r\n") {
					return fmt.Errorf("ambiguous Git value")
				}
				values[k] = value
			}
			return nil
		})
		return values, err
	}
	values := map[string]string{}
	for _, k := range x.GitKeys {
		v, ok := x.GitValues[k]
		if !ok {
			return nil, fmt.Errorf("bounded Git entry requires declarative git_values for %s; Track those fields first", k)
		}
		if strings.ContainsAny(v, "\x00\r\n") {
			return nil, fmt.Errorf("invalid Git value")
		}
		values[k] = v
	}
	return values, nil
}
func hiddenState(s FileState) FileState {
	return FileState{Exists: s.Exists, Mode: s.Mode, Hash: digest(s)}
}
func (e *Engine) addGitAction(ctx context.Context, p *Plan, x Entry) error {
	values, err := e.gitEntryValues(ctx, x)
	if err != nil {
		return err
	}
	before, err := readFile(e.config.Home, ".gitconfig")
	if err != nil {
		return err
	}
	after, err := e.editGit(ctx, before, values)
	if err != nil {
		return err
	}
	a := Action{Root: "home", Path: ".gitconfig", Before: hiddenState(before), After: hiddenState(after), GitValues: values}
	if before.Exists && digest(before) != digest(after) {
		a.BackupPath = ".gitconfig.dot-backup"
		backup, err := readFile(e.config.Home, a.BackupPath)
		if err != nil {
			return err
		}
		a.BackupBefore = hiddenState(backup)
		p.Gates.Backup = p.Gates.Backup || backup.Exists
	}
	for _, old := range p.Actions {
		if old.Root == a.Root && (old.Path == a.Path || old.BackupPath == a.Path || a.BackupPath != "" && old.Path == a.BackupPath) {
			return fmt.Errorf("bounded Git action conflicts with exact file/backup")
		}
	}
	p.Actions = append(p.Actions, a)
	return nil
}
