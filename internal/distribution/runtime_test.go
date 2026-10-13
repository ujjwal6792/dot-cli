package distribution_test

import (
	"context"
	"github.com/ujjwal6792/dots-cli/internal/distribution"
	"strings"
	"testing"
)

func TestPackagedRuntimeNeverFallsBackToSystemDependency(t *testing.T) {
	_, err := distribution.Resolve(context.Background(), "1.2.3", "", "")
	if err == nil || !strings.Contains(err.Error(), "release") {
		t.Fatalf("missing release should fail closed, got %v", err)
	}
}
