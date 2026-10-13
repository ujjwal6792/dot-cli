package setup

import (
	"context"
	"encoding/json"
	"fmt"
	"os"
	"os/exec"
	"regexp"
	"strings"

	"github.com/ujjwal6792/dots-cli/internal/native"
	"github.com/ujjwal6792/dots-cli/internal/tui"
)

type command struct {
	Path string   `json:"path"`
	Args []string `json:"args"`
	Hash string   `json:"hash"`
}

func displayCommand(c command) string {
	words := append([]string{c.Path}, c.Args...)
	for i, w := range words {
		if !regexp.MustCompile(`^[a-zA-Z0-9_./:@%+=,-]+$`).MatchString(w) {
			words[i] = "'" + strings.ReplaceAll(w, "'", "'\"'\"'") + "'"
		}
	}
	return strings.Join(words, " ")
}

type dependencyPlan struct {
	ID          string            `json:"id"`
	Commands    []command         `json:"commands"`
	Missing     []string          `json:"missing"`
	Paths       map[string]string `json:"paths"`
	Environment string            `json:"environment"`
	UID         int               `json:"uid"`
}

func binary(path, name string) (string, error) {
	if path == "" {
		path = name
	}
	return exec.LookPath(path)
}
func binaryHash(path string) (string, error) {
	b, err := os.ReadFile(path)
	if err != nil {
		return "", err
	}
	return hash(b), nil
}
func managerEnv() []string {
	env := []string{}
	for _, v := range os.Environ() {
		if !strings.HasPrefix(v, "HOMEBREW_NO_ASK=") {
			env = append(env, v)
		}
	}
	return append(env, "HOMEBREW_NO_AUTO_UPDATE=1", "HOMEBREW_NO_INSTALL_CLEANUP=1", "HOMEBREW_NO_INSTALLED_DEPENDENTS_CHECK=1", "HOMEBREW_NO_INSTALL_UPGRADE=1", "LC_ALL=C")
}
func (e *Engine) manager() (string, string, error) {
	name, path := "brew", e.options.Brew
	if e.options.OS == "linux" {
		name, path = "pacman", e.options.Pacman
	}
	resolved, err := binary(path, name)
	if err != nil {
		return name, "", fmt.Errorf("%s is unavailable; install tools manually from official links and refresh", name)
	}
	return name, resolved, nil
}
func (e *Engine) dependencies(ctx context.Context) (tui.DependencyResult, dependencyPlan, error) {
	out := tui.DependencyResult{Ready: true}
	p := dependencyPlan{Missing: []string{}, Commands: []command{}, Paths: map[string]string{}}
	p.Environment = hash(encode(managerEnv()))
	p.UID = os.Geteuid()
	for _, tool := range []struct{ name, path, link, prefix string }{
		{"git", e.options.Git, "https://git-scm.com/", "git version "},
		{"chezmoi", e.options.Chezmoi, "https://www.chezmoi.io/", "chezmoi version "},
	} {
		resolved, err := binary(tool.path, tool.name)
		p.Paths[tool.name] = resolved
		if err == nil {
			c := exec.CommandContext(ctx, resolved, "--version")
			b, er := c.Output()
			if er == nil && strings.HasPrefix(string(b), tool.prefix) {
				continue
			}
			out.Rows = append(out.Rows, tui.DependencyRow{Tool: tool.name, Link: tool.link, Problem: "existing executable is incompatible; repair manually (never overwritten)"})
			out.Ready = false
			continue
		}
		p.Missing = append(p.Missing, tool.name)
		out.Ready = false
		out.Rows = append(out.Rows, tui.DependencyRow{Tool: tool.name, Link: tool.link, Problem: "missing"})
	}
	if len(p.Missing) > 0 {
		name, path, err := e.manager()
		out.Manager = name
		if err != nil {
			for i := range out.Rows {
				out.Rows[i].Command = "Manual install; manager unavailable"
			}
		} else {
			args := append([]string{"install", "--formula", "--"}, p.Missing...)
			if name == "pacman" {
				args = append([]string{"-S", "--needed", "--"}, p.Missing...)
			}
			h, err := binaryHash(path)
			if err != nil {
				return out, p, err
			}
			cmd := command{Path: path, Args: args, Hash: h}
			if name == "pacman" && os.Geteuid() != 0 {
				sudo, err := binary(e.options.Sudo, "sudo")
				if err != nil {
					return out, p, fmt.Errorf("sudo missing; no privilege prompt fallback")
				}
				cmd.Path = sudo
				cmd.Args = append([]string{path}, args...)
				cmd.Hash, err = binaryHash(sudo)
				if err != nil {
					return out, p, err
				}
				p.Paths["pacman"] = path + ":" + h
			}
			p.Commands = append(p.Commands, cmd)
			exact := displayCommand(cmd)
			for i := range out.Rows {
				if out.Rows[i].Problem == "missing" {
					out.Rows[i].Command = exact
				}
			}
		}
	}
	p.ID = hash(encode(p))
	b, _ := json.Marshal(p)
	_ = json.Unmarshal(b, &out.Plan)
	return out, p, nil
}
func (e *Engine) InitDependencies(ctx context.Context) (tui.DependencyResult, error) {
	out, _, err := e.dependencies(ctx)
	return out, err
}
func (e *Engine) InstallDependencies(ctx context.Context, plan map[string]any, id string, streams tui.TerminalStreams) error {
	var p dependencyPlan
	b, _ := json.Marshal(plan)
	if err := native.Decode(b, &p); err != nil {
		return fmt.Errorf("invalid dependency plan")
	}
	if id == "" || id != p.ID {
		return fmt.Errorf("exact dependency install approval required")
	}
	out, fresh, err := e.dependencies(ctx)
	if err != nil {
		return err
	}
	if fresh.ID != p.ID {
		return fmt.Errorf("stale dependency plan; refresh")
	}
	if len(fresh.Commands) == 0 {
		return fmt.Errorf("no approved manager installation available; install manually and refresh")
	}
	if out.Manager == "brew" && os.Geteuid() == 0 {
		return fmt.Errorf("Homebrew must run as its unprivileged owner")
	}
	for _, cmd := range fresh.Commands {
		if err := runTerminal(ctx, cmd, streams); err != nil {
			return err
		}
	}
	out, _, err = e.dependencies(ctx)
	if err != nil {
		return err
	}
	if !out.Ready {
		return fmt.Errorf("required dependencies still missing/incompatible; setup incomplete")
	}
	return nil
}
func runTerminal(ctx context.Context, c command, s tui.TerminalStreams) error {
	if s.In == nil || s.Out == nil || s.Err == nil {
		return fmt.Errorf("native terminal streams required; no raw-tty fallback")
	}
	h, err := binaryHash(c.Path)
	if err != nil {
		return err
	}
	if h != c.Hash {
		return fmt.Errorf("manager executable changed; review again")
	}
	cmd := exec.CommandContext(ctx, c.Path, c.Args...)
	cmd.Env = managerEnv()
	cmd.Stdin = s.In
	cmd.Stdout = s.Out
	cmd.Stderr = s.Err
	if err := cmd.Run(); err != nil {
		return fmt.Errorf("manager interrupted or failed: %w", err)
	}
	return nil
}
