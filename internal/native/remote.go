package native

import (
	"context"
	"fmt"
	"net/url"
	"os"
	"path/filepath"
	"strings"
)

func safeOrigin(s string) error {
	if s == "" || strings.ContainsAny(s, "\x00\r\n\t ?#") || strings.HasPrefix(s, "-") {
		return fmt.Errorf("unsafe origin; credentials/query/fragments are forbidden")
	}
	if filepath.IsAbs(s) {
		return nil
	} // Local bare repositories are legitimate offline remotes.
	if strings.Contains(s, "://") {
		u, err := url.Parse(s)
		if err != nil || (u.Scheme != "https" && u.Scheme != "ssh") || u.Host == "" || u.RawQuery != "" || u.Fragment != "" {
			return fmt.Errorf("unsupported origin URL")
		}
		if u.User != nil && (u.Scheme != "ssh" || strings.Contains(u.User.String(), ":")) {
			return fmt.Errorf("credential-bearing origin rejected")
		}
		return nil
	}
	// Git's SCP-style SSH syntax, not arbitrary remote helper transports.
	parts := strings.SplitN(s, ":", 2)
	if len(parts) != 2 || parts[0] == "" || parts[1] == "" || strings.Contains(parts[0], "/") || strings.Contains(s, "::") {
		return fmt.Errorf("unsupported origin transport")
	}
	return nil
}
func (e *Engine) remote(ctx context.Context, operation string, confirmed bool) error {
	if !confirmed {
		return fmt.Errorf("explicit %s consent required", operation)
	}
	if _, err := e.CheckContract(ctx); err != nil {
		return err
	}
	if _, err := e.LoadAdoption(); err != nil {
		return err
	}
	unlock, err := e.lock()
	if err != nil {
		return err
	}
	defer unlock()
	if _, err := os.Lstat(filepath.Join(e.config.State, "recovery.json")); err == nil {
		return fmt.Errorf("recover interrupted operation before remote changes")
	} else if !os.IsNotExist(err) {
		return err
	}
	b, err := e.git(ctx, "remote", "get-url", "origin")
	if err != nil {
		return err
	}
	if err = safeOrigin(strings.TrimSpace(string(b))); err != nil {
		return err
	}
	b, err = e.git(ctx, "symbolic-ref", "--quiet", "--short", "HEAD")
	if err != nil {
		return fmt.Errorf("remote operations require a named branch")
	}
	branch := strings.TrimSpace(string(b))
	if operation == "pull" {
		b, err = e.git(ctx, "status", "--porcelain")
		if err != nil {
			return err
		}
		if len(b) > 0 {
			return fmt.Errorf("checkout has dirty/untracked work; preserve and review it before pull")
		}
		_, err = e.git(ctx, "-c", "core.hooksPath=/dev/null", "-c", "protocol.ext.allow=never", "pull", "--ff-only", "--no-rebase", "origin", branch)
		if err != nil {
			return err
		}
		_, err = e.CheckContract(ctx)
		return err
	}
	pushURL, err := e.git(ctx, "remote", "get-url", "--push", "origin")
	if err != nil {
		return err
	}
	if err = safeOrigin(strings.TrimSpace(string(pushURL))); err != nil {
		return err
	}
	_, err = e.git(ctx, "-c", "core.hooksPath=/dev/null", "-c", "protocol.ext.allow=never", "push", "origin", "HEAD:refs/heads/"+branch)
	return err
}

// Pull changes only the repository using a fast-forward; no apply/commit/stash/reset.
func (e *Engine) Pull(ctx context.Context, confirmed bool) error {
	return e.remote(ctx, "pull", confirmed)
}

// Push publishes only commits already made explicitly by the user, never working files.
func (e *Engine) Push(ctx context.Context, confirmed bool) error {
	return e.remote(ctx, "push", confirmed)
}
