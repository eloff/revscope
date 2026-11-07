/** Sets a cookie for jsdom's page, as a raw `name=value; attributes` string. */
export function setCookie(cookie: string): void {
  // biome-ignore lint/suspicious/noDocumentCookie: tests stand in for what the code under test writes.
  document.cookie = cookie;
}

export function clearCookie(name: string): void {
  setCookie(`${name}=; Max-Age=0; Path=/`);
}
