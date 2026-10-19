# Research

Source material the OpenScion design draws on. These notes describe other
systems as they are; what OpenScion does is defined in
[`../specs/`](../specs/README.md). Each note ends with a section on what it
means for OpenScion.

| Note | What it covers |
| --- | --- |
| [Scion](scion.md) | Anthropic's internal system OpenScion is modelled on: principles, changesets, tiers, results |
| [git internals](git-internals.md) | objects and hashing, packfiles, bitmaps, commit-graph, refs and reftable, the index, history algorithms |
| [git transfer and scaling](git-transfer-and-scaling.md) | protocol v2, negotiation, shallow and partial clone, Scalar and VFS for Git, Piper, Sapling and Mononoke |
| [GitHub](github.md) | Spokes replication, fork networks, maintenance, pull requests, merge queue, the git database API |
| [GitLab](gitlab.md) | Gitaly and Praefect, object pools, merge requests, merge trains, the Commits API |
| [storage persistence](storage-persistence.md) | phase-4 source material: the two-port design, how git/Mononoke/jj/Piper and peer Scion clones persist, and the filesystem + SQLite recommendation |
