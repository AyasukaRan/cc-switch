const RELEASES_URL = "https://github.com/AyasukaRan/cc-switch/releases";

export function releaseNotesUrl(version?: string | null): string {
  const number = version?.replace(/^v/, "");
  return number
    ? `${RELEASES_URL}/tag/ran-v${encodeURIComponent(number)}`
    : RELEASES_URL;
}
