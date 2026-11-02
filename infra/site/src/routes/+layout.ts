// Every page is prerendered into plain static files (adapter-static). URLs have no trailing slash; the files are
// written as name.html, which the Cloudflare assets setting html_handling = "auto-trailing-slash" serves at /name.
export const prerender = true;
export const trailingSlash = "never";
