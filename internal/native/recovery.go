package native

import (
	"context"
	"fmt"
)

// Journal is private target-local recovery data. Public plans contain only hashes
// and selected Git fields, never unrelated credentials from the target Git file.
type Journal struct {
	Plan    Plan     `json:"plan"`
	Actions []Action `json:"actions"`
}

func (e *Engine) materialize(ctx context.Context, p Plan) (Journal, error) {
	j := Journal{Plan: p, Actions: append([]Action{}, p.Actions...)}
	for i, a := range j.Actions {
		if a.GitValues != nil {
			before, err := readFile(e.config.Home, a.Path)
			if err != nil {
				return j, err
			}
			if digest(hiddenState(before)) != digest(a.Before) {
				return j, fmt.Errorf("Git target changed")
			}
			after, err := e.editGit(ctx, before, a.GitValues)
			if err != nil {
				return j, err
			}
			j.Actions[i].Before = before
			j.Actions[i].After = after
			if a.BackupPath != "" {
				backup, err := readFile(e.config.Home, a.BackupPath)
				if err != nil {
					return j, err
				}
				if digest(hiddenState(backup)) != digest(a.BackupBefore) {
					return j, fmt.Errorf("Git backup changed")
				}
				j.Actions[i].BackupBefore = backup
			}
		}
	}
	return j, nil
}
func (e *Engine) Recovery() (Journal, error) {
	var j Journal
	err := readJSON(e.config.State, "recovery.json", &j)
	return j, err
}

// Recover rolls back only the reviewed exact actions and refuses to overwrite
// post-interruption edits. It never retries an apply or silently broadens scope.
func (e *Engine) Recover(ctx context.Context, approve string) error {
	unlock, err := e.lock()
	if err != nil {
		return err
	}
	defer unlock()
	j, err := e.Recovery()
	if err != nil {
		return err
	}
	p := j.Plan
	if approve == "" || approve != p.ID || p.ID != planID(p) || len(j.Actions) != len(p.Actions) {
		return fmt.Errorf("exact recovery approval required")
	}
	c, err := e.CheckContract(ctx)
	if err != nil {
		return err
	}
	if err = e.validateRecoveryScope(p, c); err != nil {
		return err
	}
	a, err := e.LoadAdoption()
	if err != nil {
		return err
	}
	if a.Machine != p.Machine {
		return fmt.Errorf("recovery machine mismatch")
	}
	for i, action := range j.Actions {
		original := p.Actions[i]
		check := action
		if original.GitValues != nil {
			check.Before = hiddenState(action.Before)
			check.After = hiddenState(action.After)
			if action.BackupPath != "" {
				check.BackupBefore = hiddenState(action.BackupBefore)
			}
		}
		if digest(check) != digest(original) {
			return fmt.Errorf("malformed recovery actions")
		}
		root, err := e.root(action.Root)
		if err != nil {
			return err
		}
		live, err := readFile(root, action.Path)
		if err != nil {
			return err
		}
		if digest(live) != digest(action.Before) && digest(live) != digest(action.After) {
			return fmt.Errorf("recovery conflicts with changed target %s", action.Path)
		}
		if action.BackupPath != "" {
			backup, err := readFile(root, action.BackupPath)
			if err != nil {
				return err
			}
			if digest(backup) != digest(action.BackupBefore) && digest(backup) != digest(action.Before) {
				return fmt.Errorf("recovery conflicts with changed backup")
			}
		}
	}
	for i := len(j.Actions) - 1; i >= 0; i-- {
		if err := ctx.Err(); err != nil {
			return err
		}
		action := j.Actions[i]
		root, _ := e.root(action.Root)
		if err := replace(root, action.Path, action.Before); err != nil {
			return err
		}
		if action.BackupPath != "" {
			if err := replace(root, action.BackupPath, action.BackupBefore); err != nil {
				return err
			}
		}
	}
	return replace(e.config.State, "recovery.json", FileState{})
}

func (e *Engine) validateRecoveryScope(p Plan, c Contract) error {
	if p.Version != 1 || p.Request.Version != 1 {
		return fmt.Errorf("incompatible recovery protocol")
	}
	allowed := map[string]bool{}
	switch p.Request.Mode {
	case "adopt", "update":
		allowed["state:adoption.json"] = true
	case "track", "metadata", "untrack":
		allowed["source:"+c.Manifest] = true
	default:
		return fmt.Errorf("unknown recovery operation")
	}
	for _, x := range p.Selected {
		if err := e.validateEntry(x); err != nil {
			return err
		}
		if !within(e.config.Source+"/"+x.Source, e.config.Source+"/"+c.HomeSource) {
			return fmt.Errorf("recovery source outside home boundary")
		}
		if p.Request.Mode == "adopt" || p.Request.Mode == "update" {
			allowed["home:"+x.Destination] = true
			for _, path := range x.Removals {
				allowed["home:"+path] = true
			}
			for _, integration := range x.Integrations {
				mirror := ""
				if integration == "gemini-skills" {
					mirror = ".gemini/skills/"
				}
				if integration == "claude-skills" {
					mirror = ".claude/skills/"
				}
				if mirror != "" {
					const prefix = ".agents/skills/"
					if len(x.Destination) <= len(prefix) || x.Destination[:len(prefix)] != prefix {
						return fmt.Errorf("invalid recovery skill mapping")
					}
					allowed["home:"+mirror+x.Destination[len(prefix):]] = true
				}
			}
		} else if p.Request.Mode == "track" {
			allowed["source:"+x.Source] = true
		}
	}
	seen := map[string]bool{}
	for _, action := range p.Actions {
		key := action.Root + ":" + action.Path
		if !allowed[key] || seen[key] {
			return fmt.Errorf("recovery action outside exact selected scope")
		}
		seen[key] = true
		if action.Root == "home" {
			if err := e.targetPath(action.Path); err != nil {
				return err
			}
		}
		if action.BackupPath != "" && (action.BackupPath != action.Path+".dot-backup" || action.Root == "state") {
			return fmt.Errorf("unbounded recovery backup")
		}
		if action.Root == "state" {
			var next Adoption
			if err := Decode(action.After.Data, &next); err != nil {
				return err
			}
			if next.Version != 3 || next.Machine != p.Machine || digest(next.Entries) != digest(p.Selected) {
				return fmt.Errorf("recovery approval/machine mismatch")
			}
		}
	}
	return nil
}
