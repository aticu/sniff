# Sniff

A tool for investigating file systems and folder contents and their changes.

Sniff can

- create snapshots of file systems and folders, storing hashes and metadata.
- compare two snapshots, looking at the changes between them.
- list files and look at metadata in a snapshot.
- aggregate data from multiple snapshots into a database.
- compare the database to a snapshot to see which files have never been seen.

## Building

Run

```text
cargo build --release
```

using a current nightly Rust release.

The resulting binary will be in `target/release/sniff`.

## Running

Run

```text
sniff --help
```

for a list of available subcommands and options.

## Creating a snapshot

To create a snapshot of a VDI image, run something like the following:

```text
sniff create-snapshot /path/to/image.vdi /output/folder
```

Note that for VDI support the command `vboximg-mount` needs to be installed.
Also currently the contained file system is a assumed to be NTFS and thus `ntfs-3g` also needs to be installed.

Optionally you may wish to record hashes and paths in the snapshot in a database file, you can do so using the following options: `-D /path/to/database/file.sqlite --comment "image description"`.

If you want to record a snapshot of a folder that is already mounted, you can simply point to that folder instead:

```text
sniff create-snapshot /path/to/folder/of/interest /output/folder
```

## Looking through a snapshot and comparing snapshots

The main subcommand that is useful here is

```text
sniff ls /path/to/snapshot /path/to/folder/in/snapshot
```

To compare how a folder changed in snapshots, use the `-c /path/to/other/snapshot` option.

To not show too much information at once, you can limit the depths of directories which are traversed with the `-d` option.

Another useful option is `-o /path/to/output/image.png`, that generates a visualization of the target folder in the snapshot.

To learn more about the other possible options use the `--help` flag.

### Example usage

```text
sniff ls ~/snapshots/my_system.snp -c ~/snapshots/my_system_later.snp /Windows -o ~/diff.png -d2 -e dll,exe -u
```

The above command compares the `Windows` folder (note the use of `/` instead of `\`) in the given snapshot with a later snapshot of the same system, taking into account the database of known files.
It will display only dll and exe files and it will only look two directories deep, after which it will just summarize how many files would have been shown.
Changed, added or removed files will be highlighted, but unchanged files will also be shown.

Also a visual representation of the differences will be generated in `~/diff.png`.
The added files will be displayed in blue, the removed files in red and the changed files in yellow.
All unchanged dll files will be displayed in cyan and all unchanged exe files will be displayed in green.
Other unchanged files will be displayed in grey.

```text
sniff ls ~/snapshots/my_system.snp -D ~/snapshot_db.sqlite / -o ~/summary.png -d1 -A 2022-12 -B 2023
```

The above command summarizes how many "unknown" files that were either accessed, modified or created in December of 2022 are in each top level folder in the snapshot.

Also a visual representation of the file system will be generated in `~/summary.png`, highlighting the known files in white and the other files in grey.

```text
sniff ls ~/snapshots/my_system.snp /path/to/some/file
```

Displays every piece of information available on the given file in detail, including hashes, the first bytes and metadata.
The same is possible for folders as seen below.

```text
sniff ls ~/snapshots/my_system.snp /path/to/some/folder -d0
```

Displays every piece of information (except for the content of the folder) available on the given folder in detail.

## Things to watch out for

- Sniff is still in the prototyping stage and thus breaking changes may occur at any time (though care will be taken to always be able to read old snapshots).
- Sniff was primarily designed to observe Windows systems stored on NTFS, but through later additions it can also work well on any UNIX system.
- Sniff in its current design can only be compiled for UNIX systems. Eventually support for other systems may be possible, but a good solution for different representations of paths on different platforms needs to be found.
- Keep in mind that if you want to look at a whole file system, a lot of data is being generated and hashed. While sniff tries to be fast and efficient, it was not designed with weak hardware in mind, so a decently performant system is recommended.

## Naming

Sniff stands for **SN**apshot creation and d**IFF**erence calculation, it used to be called **SN**apd**IFF** in earlier iterations.
