#!/usr/bin/env python3
import urllib.request
import json
import sys
import argparse

try:
    import websocket
except ImportError:
    print("Error: The 'websocket-client' library is required. Please install it first:", file=sys.stderr)
    print("  pip install websocket-client", file=sys.stderr)
    sys.exit(1)

def get_page_websocket_url(target_domain, port=9222):
    list_url = f"http://127.0.0.1:{port}/json/list"
    print(f"Fetching open tabs from Chrome at {list_url}...", file=sys.stderr)
    try:
        req = urllib.request.Request(list_url)
        with urllib.request.urlopen(req) as response:
            tabs = json.loads(response.read().decode())
            if not tabs:
                print("Error: No active tabs found in Chrome. Make sure you have at least one tab open.", file=sys.stderr)
                sys.exit(1)
            
            # Find matching tab
            target_tab = None
            for tab in tabs:
                url = tab.get("url", "").lower()
                title = tab.get("title", "").lower()
                if target_domain in url or target_domain in title:
                    target_tab = tab
                    break
            
            if not target_tab:
                # Fallback to first page
                page_tabs = [t for t in tabs if t.get("type") == "page"]
                if page_tabs:
                    target_tab = page_tabs[0]
                    print(f"Tab for domain '{target_domain}' not found. Falling back to first available page...", file=sys.stderr)
            
            if not target_tab:
                print("Error: No active page tabs found in Chrome.", file=sys.stderr)
                sys.exit(1)
                
            ws_url = target_tab.get("webSocketDebuggerUrl")
            if not ws_url:
                print("Error: Could not find webSocketDebuggerUrl for the target tab.", file=sys.stderr)
                sys.exit(1)
                
            print(f"Connecting to Page WebSocket for tab '{target_tab.get('title')}' ({target_tab.get('url')})", file=sys.stderr)
            return ws_url
            
    except Exception as e:
        print(f"Error: Failed to connect to Chrome at {list_url}. Make sure Chrome is running with remote debugging enabled.", file=sys.stderr)
        print("\nTo start Chrome with remote debugging, run this command in Windows Terminal / PowerShell:", file=sys.stderr)
        print('  Start-Process "chrome.exe" -ArgumentList "--remote-debugging-port=9222"', file=sys.stderr)
        print(f"\nDetails: {e}", file=sys.stderr)
        sys.exit(1)

def get_cookies(ws_url):
    ws = websocket.create_connection(ws_url)
    try:
        # Get cookies directly on the page websocket
        print("Fetching cookies from the page target...", file=sys.stderr)
        ws.send(json.dumps({
            "id": 123,
            "method": "Network.getCookies",
            "params": {}
        }))
        
        response = json.loads(ws.recv())
        if "error" in response:
            print(f"CDP Error: {response['error']}", file=sys.stderr)
            sys.exit(1)
            
        return response.get("result", {}).get("cookies", [])
    finally:
        ws.close()

def main():
    parser = argparse.ArgumentParser(description="Fetch cookies from a running Chrome instance using CDP.")
    parser.add_argument("domain", help="The domain to fetch cookies for (e.g., xiaomimimo.com)")
    parser.add_argument("--port", type=int, default=9222, help="Chrome remote debugging port (default: 9222)")
    args = parser.parse_args()

    target_domain = args.domain.lower().strip().lstrip('.')

    ws_url = get_page_websocket_url(target_domain, args.port)
    cookies = get_cookies(ws_url)

    if not cookies:
        print("No cookies found in the page context.", file=sys.stderr)
        sys.exit(0)

    # Filter cookies by domain
    filtered_cookies = []
    for c in cookies:
        cookie_domain = c.get("domain", "").lower().lstrip('.')
        if target_domain in cookie_domain or cookie_domain in target_domain:
            filtered_cookies.append(c)

    if not filtered_cookies:
        print(f"No cookies found matching domain '{args.domain}'.", file=sys.stderr)
        sys.exit(0)

    # Format into Cookie Header String
    cookie_str = "; ".join([f"{c['name']}={c['value']}" for c in filtered_cookies])
    
    print("\n" + "="*10 + f" Cookies for {args.domain} " + "="*10)
    print("\n[Cookie Header String]:")
    print(cookie_str)
    
    print("\n[JSON Details (First 3 shown)]:")
    print(json.dumps(filtered_cookies[:3], indent=2))

if __name__ == "__main__":
    main()
