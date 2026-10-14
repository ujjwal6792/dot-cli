package setup

import (
	"context"
	"encoding/json"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"regexp"
	"sort"
	"strings"
	"time"
	"unicode/utf8"

	"github.com/ujjwal6792/dots-cli/internal/native"
	"github.com/ujjwal6792/dots-cli/internal/tui"
)

var _ tui.PackagesEngine = (*Engine)(nil)
var _ tui.PackageTerminalEngine = (*Engine)(nil)

type packagePlan struct {
	ID          string           `json:"id"`
	Source      string           `json:"source"`
	Manager     string           `json:"manager"`
	Digest      string           `json:"digest"`
	Exists      bool             `json:"exists"`
	Rows        []tui.PackageRow `json:"rows"`
	Commands    []command        `json:"commands"`
	ManagerHash string           `json:"manager_hash"`
	Environment string           `json:"environment"`
	UID         int              `json:"uid"`
}

func packageName(name string, brew bool) bool {
	if !brew {
		return regexp.MustCompile(`^[a-z0-9][a-z0-9+_.-]*$`).MatchString(name)
	}
	parts := strings.Split(name, "/")
	if len(parts) != 1 && len(parts) != 3 {
		return false
	}
	for _, p := range parts[:len(parts)-1] {
		if !regexp.MustCompile(`^[a-z0-9][a-z0-9+_.-]*$`).MatchString(p) {
			return false
		}
	}
	return regexp.MustCompile(`^[a-z0-9][a-z0-9+_-]*(@[0-9][a-z0-9+_.-]*)?$`).MatchString(parts[len(parts)-1]) && !strings.HasSuffix(name, ".rb") && !strings.HasSuffix(name, ".json")
}
func parsePackages(manager, text string) ([]tui.PackageRow, []string) {
	rows := []tui.PackageRow{}
	diagnostics := []string{}
	seen := map[string]bool{}
	pattern := regexp.MustCompile(`^\s*(brew|cask)\s+("([^"]+)"|'([^']+)')\s*(#.*)?$`)
	for i, line := range strings.Split(text, "\n") {
		line = strings.TrimSpace(line)
		if line == "" || strings.HasPrefix(line, "#") {
			continue
		}
		kind, name := manager, strings.TrimSpace(strings.SplitN(line, "#", 2)[0])
		valid := true
		if manager == "brew" {
			m := pattern.FindStringSubmatch(line)
			if m == nil {
				valid = false
			} else {
				kind, name = m[1], m[3]
				if name == "" {
					name = m[4]
				}
			}
		}
		if !valid || !packageName(name, manager == "brew") || seen[kind+"/"+name] {
			diagnostics = append(diagnostics, fmt.Sprintf("line %d: unsupported/duplicate syntax; install blocked", i+1))
			continue
		}
		seen[kind+"/"+name] = true
		rows = append(rows, tui.PackageRow{Manager: manager, Kind: kind, Name: name, Desired: true})
	}
	return rows, diagnostics
}
func (e *Engine) packagePath(manager string) (string, error) {
	file := "Brewfile"
	if manager == "pacman" {
		file = "pacman.txt"
	} else if manager != "brew" {
		return "", fmt.Errorf("unsupported package manager (only brew/pacman)")
	}
	if (e.options.OS == "darwin") != (manager == "brew") {
		return "", fmt.Errorf("manager conflicts with OS")
	}
	return native.BoundedPath(e.options.Source, "packages/"+file)
}
func readPackageText(path string) (string, bool, error) {
	i, err := os.Lstat(path)
	if os.IsNotExist(err) {
		return "", false, nil
	}
	if err != nil {
		return "", false, err
	}
	if !i.Mode().IsRegular() || i.Size() > 1024*1024 {
		return "", false, fmt.Errorf("package source must be bounded regular UTF-8 text under 1 MiB")
	}
	b, err := os.ReadFile(path)
	if err != nil {
		return "", false, err
	}
	if !utf8.Valid(b) || strings.ContainsRune(string(b), 0) {
		return "", false, fmt.Errorf("package source must be UTF-8 text")
	}
	return string(b), true, nil
}
func (e *Engine) packageSource(manager string) (tui.PackageSource, []tui.PackageRow, error) {
	p, err := e.packagePath(manager)
	if err != nil {
		return tui.PackageSource{}, nil, err
	}
	text, exists, err := readPackageText(p)
	if err != nil {
		return tui.PackageSource{}, nil, err
	}
	rows, diagnostics := parsePackages(manager, text)
	return tui.PackageSource{Text: text, Digest: hash([]byte(text)), Exists: exists, Diagnostics: diagnostics}, rows, nil
}
func managerOutput(ctx context.Context, path string, args ...string) (string, error) {
	cmd := exec.CommandContext(ctx, path, args...)
	cmd.Env = managerEnv()
	b, err := cmd.Output()
	if err != nil {
		return "", fmt.Errorf("package installed-state query failed; no missing state assumed")
	}
	return strings.TrimSpace(string(b)), nil
}
func (e *Engine) packageStatus(ctx context.Context) (tui.PackageResult, string, error) {
	manager, path, managerErr := e.manager()
	src, rows, err := e.packageSource(manager)
	if err != nil {
		return tui.PackageResult{}, "", err
	}
	out := tui.PackageResult{Platform: e.options.OS, Sources: map[string]tui.PackageSource{manager: src}}
	installed := map[string]tui.PackageRow{}
	if managerErr == nil {
		if manager == "brew" {
			for _, pair := range []struct{ kind, flag string }{{"brew", "--formula"}, {"cask", "--cask"}} {
				text, err := managerOutput(ctx, path, "list", pair.flag, "--full-name")
				if err != nil {
					managerErr = err
					break
				}
				for _, name := range strings.Fields(text) {
					if !packageName(name, true) {
						managerErr = fmt.Errorf("invalid installed package response")
						break
					}
					installed[pair.kind+"/"+name] = tui.PackageRow{Manager: manager, Kind: pair.kind, Name: name, Installed: true}
				}
			}
		} else {
			text, err := managerOutput(ctx, path, "-Qq")
			if err != nil {
				managerErr = err
			} else {
				for _, name := range strings.Fields(text) {
					if !packageName(name, false) {
						managerErr = fmt.Errorf("invalid installed package response")
						break
					}
					installed[manager+"/"+name] = tui.PackageRow{Manager: manager, Kind: manager, Name: name, Installed: true}
				}
			}
		}
	}
	for i := range rows {
		if managerErr != nil {
			rows[i].Status = "unavailable"
			continue
		}
		key := rows[i].Kind + "/" + rows[i].Name
		if manager == "brew" && (strings.HasPrefix(rows[i].Name, "homebrew/core/") || strings.HasPrefix(rows[i].Name, "homebrew/cask/")) {
			key = rows[i].Kind + "/" + filepath.Base(rows[i].Name)
		}
		_, ok := installed[key]
		if !ok && manager == "brew" && !strings.Contains(rows[i].Name, "/") {
			matches := []string{}
			for k, r := range installed {
				if r.Kind == rows[i].Kind && filepath.Base(r.Name) == rows[i].Name {
					matches = append(matches, k)
				}
			}
			if len(matches) == 1 {
				key = matches[0]
				ok = true
			} else if len(matches) > 1 {
				rows[i].Status = "unavailable"
				managerErr = fmt.Errorf("ambiguous Homebrew ownership; review full package names")
				continue
			}
		}
		rows[i].Installed = ok
		rows[i].Missing = !ok
		delete(installed, key)
	}
	if managerErr == nil {
		for _, r := range installed {
			rows = append(rows, r)
		}
	} else {
		out.Notice = managerErr.Error()
	}
	sort.Slice(rows, func(i, j int) bool { return rows[i].Kind+"/"+rows[i].Name < rows[j].Kind+"/"+rows[j].Name })
	out.Rows = rows
	return out, path, nil
}
func (e *Engine) planPackages(ctx context.Context) (tui.PackageResult, packagePlan, error) {
	out, path, err := e.packageStatus(ctx)
	if err != nil {
		return out, packagePlan{}, err
	}
	if out.Notice != "" {
		return out, packagePlan{}, fmt.Errorf("package state unavailable; refresh before review")
	}
	manager, _, err := e.manager()
	if err != nil {
		return out, packagePlan{}, err
	}
	src := out.Sources[manager]
	if len(src.Diagnostics) > 0 {
		return out, packagePlan{}, fmt.Errorf("unsupported package source syntax; edit before Install")
	}
	mh, err := binaryHash(path)
	if err != nil {
		return out, packagePlan{}, err
	}
	p := packagePlan{Source: e.options.Source, Manager: manager, Digest: src.Digest, Exists: src.Exists, Rows: out.Rows, Commands: []command{}, ManagerHash: mh, Environment: hash(encode(managerEnv())), UID: os.Geteuid()}
	kinds := []string{"brew", "cask"}
	if manager == "pacman" {
		kinds = []string{"pacman"}
	}
	for _, kind := range kinds {
		names := []string{}
		for _, r := range out.Rows {
			if r.Kind == kind && r.Missing {
				names = append(names, r.Name)
			}
		}
		if len(names) == 0 {
			continue
		}
		args := []string{"install", "--formula", "--"}
		if kind == "cask" {
			args = []string{"install", "--cask", "--"}
		}
		if manager == "pacman" {
			args = []string{"-S", "--needed", "--"}
		}
		args = append(args, names...)
		cmd := command{Path: path, Args: args, Hash: mh}
		if manager == "pacman" && os.Geteuid() != 0 {
			sudo, err := binary(e.options.Sudo, "sudo")
			if err != nil {
				return out, p, fmt.Errorf("sudo unavailable; no raw password fallback")
			}
			cmd.Path = sudo
			cmd.Args = append([]string{path}, args...)
			cmd.Hash, err = binaryHash(sudo)
			if err != nil {
				return out, p, err
			}
		}
		p.Commands = append(p.Commands, cmd)
	}
	p.ID = hash(encode(p))
	_ = json.Unmarshal(encode(p), &out.Plan)
	out.Review = "Install missing · exact package plan " + p.ID + "\nPrivate desired lists stay unchanged; nothing uninstalled.\nManagers may add dependencies; inspect native transaction/prompts before authorizing.\n"
	for _, c := range p.Commands {
		out.Review += "Command: " + displayCommand(c) + "\n"
	}
	if len(p.Commands) == 0 {
		out.Review += "Nothing missing; no installation will run.\n"
	}
	if manager == "pacman" {
		out.Review += "No system upgrade requested. Arch partial upgrades are unsupported; maintain system/repository synchronization separately.\n"
	}
	return out, p, nil
}
func (e *Engine) savePackages(manager, digest, text string) (tui.PackageResult, error) {
	if len(text) > 1024*1024 || !utf8.ValidString(text) || strings.ContainsRune(text, 0) {
		return tui.PackageResult{}, fmt.Errorf("package source must be bounded UTF-8 text")
	}
	old, _, err := e.packageSource(manager)
	if err != nil {
		return tui.PackageResult{}, err
	}
	if old.Digest != digest {
		return tui.PackageResult{}, fmt.Errorf("stale source; reload before saving")
	}
	path, err := e.packagePath(manager)
	if err != nil {
		return tui.PackageResult{}, err
	}
	if err := os.MkdirAll(filepath.Dir(path), 0700); err != nil {
		return tui.PackageResult{}, err
	}
	if old.Exists {
		backup, err := native.BoundedPath(e.options.Source, "packages/.backups/"+filepath.Base(path)+"."+time.Now().UTC().Format("20060102T150405.000000000Z"))
		if err != nil {
			return tui.PackageResult{}, err
		}
		if err := exclusive(backup, []byte(old.Text)); err != nil {
			return tui.PackageResult{}, err
		}
	}
	f, err := os.CreateTemp(filepath.Dir(path), ".package-edit-")
	if err != nil {
		return tui.PackageResult{}, err
	}
	defer os.Remove(f.Name())
	_, err = f.WriteString(text)
	if err == nil {
		err = f.Sync()
	}
	ce := f.Close()
	if err != nil {
		return tui.PackageResult{}, err
	}
	if ce != nil {
		return tui.PackageResult{}, ce
	}
	latest, _, err := e.packageSource(manager)
	if err != nil {
		return tui.PackageResult{}, err
	}
	if latest.Digest != digest || latest.Exists != old.Exists {
		return tui.PackageResult{}, fmt.Errorf("stale source; reload before saving")
	}
	if err := os.Rename(f.Name(), path); err != nil {
		return tui.PackageResult{}, err
	}
	src, _, err := e.packageSource(manager)
	return tui.PackageResult{Sources: map[string]tui.PackageSource{manager: src}}, err
}
func (e *Engine) Packages(ctx context.Context, action string, request map[string]any, plan any, approve string) (tui.PackageResult, error) {
	if err := e.Continue(ctx); err != nil {
		return tui.PackageResult{}, err
	}
	switch action {
	case "status":
		out, _, err := e.packageStatus(ctx)
		return out, err
	case "plan":
		out, _, err := e.planPackages(ctx)
		return out, err
	case "save", "import":
		var r struct {
			Manager  string `json:"manager"`
			Digest   string `json:"digest"`
			Text     string `json:"text,omitempty"`
			Supplied string `json:"supplied,omitempty"`
		}
		b, _ := json.Marshal(request)
		if err := native.Decode(b, &r); err != nil {
			return tui.PackageResult{}, fmt.Errorf("invalid package edit request")
		}
		if action == "import" {
			path, err := native.BoundedPath(e.options.Source, r.Supplied)
			if err != nil {
				return tui.PackageResult{}, err
			}
			text, exists, err := readPackageText(path)
			if err != nil {
				return tui.PackageResult{}, err
			}
			if !exists {
				return tui.PackageResult{}, fmt.Errorf("import source missing")
			}
			r.Text = text
		}
		return e.savePackages(r.Manager, r.Digest, r.Text)
	case "apply":
		return tui.PackageResult{}, fmt.Errorf("package install requires separate native terminal handoff")
	}
	return tui.PackageResult{}, fmt.Errorf("unsupported package action")
}
func (e *Engine) InstallPackages(ctx context.Context, plan map[string]any, id string, s tui.TerminalStreams) error {
	var p packagePlan
	b, _ := json.Marshal(plan)
	if err := native.Decode(b, &p); err != nil {
		return fmt.Errorf("invalid package plan")
	}
	if id == "" || p.ID != id {
		return fmt.Errorf("exact package plan approval required")
	}
	if err := e.Continue(ctx); err != nil {
		return err
	}
	_, fresh, err := e.planPackages(ctx)
	if err != nil {
		return err
	}
	if fresh.ID != p.ID {
		return fmt.Errorf("stale package plan; review again")
	}
	if fresh.Manager == "brew" && os.Geteuid() == 0 {
		return fmt.Errorf("Homebrew must run as its unprivileged owner")
	}
	for _, c := range fresh.Commands {
		if err := runTerminal(ctx, c, s); err != nil {
			return err
		}
	}
	out, _, err := e.packageStatus(ctx)
	if err != nil {
		return err
	}
	if out.Notice != "" {
		return fmt.Errorf("cannot verify installed package state")
	}
	for _, r := range out.Rows {
		if r.Missing {
			return fmt.Errorf("reviewed packages remain missing; installation incomplete")
		}
	}
	return nil
}
