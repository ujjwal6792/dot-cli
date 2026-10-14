package native

import (
	"bytes"
	"context"
	"fmt"
	"path/filepath"
	"strconv"
	"strings"
	"text/template"
)

func (e *Engine) renderTemplate(x Entry, s FileState) (FileState, error) {
	body := x.Template
	if body == "" {
		body = string(s.Data)
	}
	t, err := template.New("declarative").Option("missingkey=error").Parse(body)
	if err != nil {
		return s, fmt.Errorf("unsupported/private executable template: %w", err)
	}
	a, err := e.LoadAdoption()
	if err != nil {
		return s, err
	}
	var out bytes.Buffer
	data, err := e.templateData()
	if err != nil {
		return s, err
	}
	data["machine"] = a.Machine
	data["chezmoi"] = map[string]any{"os": e.config.OS, "homeDir": e.config.Home}
	err = t.Execute(&out, data)
	if err != nil {
		return s, err
	}
	s.Data = out.Bytes()
	return s, nil
}
func (e *Engine) gitContent(ctx context.Context, x Entry) (FileState, error) {
	values, err := e.gitEntryValues(ctx, x)
	if err != nil {
		return FileState{}, err
	}
	s, err := readFile(e.config.Home, ".gitconfig")
	if err != nil {
		return s, err
	}
	return e.editGit(ctx, s, values)
}

// Only scalar string tracker data is part of this native rendering contract.
// Unsupported TOML values fail explicitly rather than evaluating private code.
func (e *Engine) templateData() (map[string]any, error) {
	data := map[string]any{}
	if e.config.TrackerConfig == "" {
		return data, nil
	}
	p := e.config.TrackerConfig
	if !filepath.IsAbs(p) {
		return nil, fmt.Errorf("tracker config must be absolute")
	}
	s, err := readFile(filepath.Dir(p), filepath.Base(p))
	if err != nil {
		return nil, err
	}
	if !s.Exists {
		return nil, fmt.Errorf("explicit tracker config is missing")
	}
	inData := false
	for _, line := range strings.Split(string(s.Data), "\n") {
		line = strings.TrimSpace(line)
		if line == "" || strings.HasPrefix(line, "#") {
			continue
		}
		if strings.HasPrefix(line, "[") {
			inData = line == "[data]"
			continue
		}
		if !inData {
			continue
		}
		parts := strings.SplitN(line, "=", 2)
		if len(parts) != 2 {
			return nil, fmt.Errorf("unsupported tracker data declaration")
		}
		key := strings.TrimSpace(parts[0])
		value := strings.TrimSpace(parts[1])
		if key == "" || strings.ContainsAny(key, ".[]\"'") {
			return nil, fmt.Errorf("unsupported tracker data key")
		}
		if _, exists := data[key]; exists {
			return nil, fmt.Errorf("duplicate tracker data key")
		}
		var parsed string
		if len(value) >= 2 && value[0] == '\'' && value[len(value)-1] == '\'' {
			parsed = value[1 : len(value)-1]
		} else {
			parsed, err = strconv.Unquote(value)
			if err != nil {
				return nil, fmt.Errorf("tracker data %s requires a scalar quoted string", key)
			}
		}
		data[key] = parsed
	}
	return data, nil
}
