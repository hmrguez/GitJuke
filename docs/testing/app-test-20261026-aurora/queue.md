# Queue edge cases

Disposable app-testing fixture.

1. Add a track while the queue is empty.
2. Add the same track twice.
3. Remove the currently selected track.
4. Skip the final track.

```text
waiting -> playing -> finished
             |
             +-> skipped
```
