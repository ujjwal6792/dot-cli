package native

import (
	"fmt"
	"os"
	"syscall"
)

// Kernel-owned locks survive neither crashes nor interrupted terminals. A stale
// lock filename is harmless; the durable recovery journal remains authoritative.
func (e *Engine) lock() (func(), error) {
	p, err := BoundedPath(e.config.State, "operation.lock")
	if err != nil {
		return nil, err
	}
	if err = os.MkdirAll(e.config.State, 0700); err != nil {
		return nil, err
	}
	fd, err := syscall.Open(p, syscall.O_CREAT|syscall.O_RDWR|syscall.O_NOFOLLOW, 0600)
	if err != nil {
		return nil, err
	}
	f := os.NewFile(uintptr(fd), p)
	if err = syscall.Flock(fd, syscall.LOCK_EX|syscall.LOCK_NB); err != nil {
		_ = f.Close()
		return nil, fmt.Errorf("another operation is in progress: %w", err)
	}
	return func() { _ = syscall.Flock(fd, syscall.LOCK_UN); _ = f.Close() }, nil
}
