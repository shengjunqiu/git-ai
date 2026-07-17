# git-ai patch

This is `interprocess` 2.4.0 with two narrowly scoped Windows fixes.

When `ReadFileEx` fails before starting an asynchronous operation, the upstream
code returned through `?` while `CannotUnwind` was still armed. Dropping that
guard aborts the process. The local patch explicitly ends the guard before
returning the immediate I/O error. Pending asynchronous operations retain the
original guard and cancellation behavior.

`PipeListener::accept` also replenishes the stored named-pipe instance and
releases its mutex before waiting for a client. This allows git-ai's fixed
worker pool to keep multiple pipe instances ready for Git Trace2 parent and
child processes that connect concurrently; the upstream ordering serialized
all acceptors and could make Git silently drop trace output when the only
ready instance was busy.
