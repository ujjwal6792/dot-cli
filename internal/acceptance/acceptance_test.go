package acceptance_test

import (
	"context"
	"github.com/ujjwal6792/dots-cli/internal/acceptance"
	"testing"
)

func TestAcceptanceRequiresRealVerifiedArtifact(t *testing.T) {
	if err := acceptance.Run(context.Background(), "missing.tar.gz", "", t.TempDir()); err == nil {
		t.Fatal("missing artifact accepted")
	}
}
