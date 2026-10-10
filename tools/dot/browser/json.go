package browser

import (
	"encoding/json"
	"fmt"
	"io"
	"strings"
)

// Syntax checking only. Policy and metadata resolution stay in the shared engine.
func uniqueJSONRequest(text string) (map[string]any, error) {
	decoder := json.NewDecoder(strings.NewReader(text))
	decoder.UseNumber()
	value, err := uniqueJSONValue(decoder)
	if err != nil {
		return nil, err
	}
	if _, err := decoder.Token(); err != io.EOF {
		return nil, fmt.Errorf("invalid trailing JSON")
	}
	request, ok := value.(map[string]any)
	if !ok {
		return nil, fmt.Errorf("request must be a JSON object")
	}
	return request, nil
}

func uniqueJSONValue(decoder *json.Decoder) (any, error) {
	token, err := decoder.Token()
	if err != nil {
		return nil, err
	}
	switch token {
	case json.Delim('{'):
		object := map[string]any{}
		for decoder.More() {
			key, err := decoder.Token()
			if err != nil {
				return nil, err
			}
			name, ok := key.(string)
			if !ok {
				return nil, fmt.Errorf("invalid JSON field")
			}
			if _, exists := object[name]; exists {
				return nil, fmt.Errorf("duplicate JSON field; keep one explicit value per layer")
			}
			value, err := uniqueJSONValue(decoder)
			if err != nil {
				return nil, err
			}
			object[name] = value
		}
		_, err := decoder.Token()
		return object, err
	case json.Delim('['):
		array := []any{}
		for decoder.More() {
			value, err := uniqueJSONValue(decoder)
			if err != nil {
				return nil, err
			}
			array = append(array, value)
		}
		_, err := decoder.Token()
		return array, err
	default:
		return token, nil
	}
}
