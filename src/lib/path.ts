/// Path helpers shared by the sidebar, file tree and chat header.
///
/// These deliberately handle BOTH separators. Workspace paths arrive from the
/// OS, so on Windows they are backslash-separated ("C:\Users\me\my-app").
/// Splitting on "/" alone never matches there, so the "last segment" ends up
/// being the entire path — which is why project labels rendered as a full
/// path instead of a folder name.

/// Last segment of a path, i.e. the file or folder's own name.
///
/// Trailing separators are ignored, so "/a/b/" yields "b". A path with no
/// separator (or an empty one) is returned unchanged rather than becoming "".
export function baseName(path: string): string {
  const parts = path.split(/[\\/]/).filter(Boolean);
  return parts.length > 0 ? parts[parts.length - 1] : path;
}
