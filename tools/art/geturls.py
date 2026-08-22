"""Resolve wiki file names to CDN URLs in batches of 50 (API limit)."""
import json, urllib.request, urllib.parse
API="https://clashofclans.fandom.com/api.php"
UA={"User-Agent":"Mozilla/5.0 (X11; Linux x86_64)"}
files=[l.strip() for l in open('files.txt') if l.strip()]
urls={}
for i in range(0,len(files),50):
    batch=files[i:i+50]
    q=urllib.parse.urlencode({"action":"query","format":"json","prop":"imageinfo",
        "iiprop":"url","titles":"|".join("File:"+f for f in batch)})
    req=urllib.request.Request(f"{API}?{q}",headers=UA)
    with urllib.request.urlopen(req,timeout=30) as r: d=json.load(r)
    for p in d.get('query',{}).get('pages',{}).values():
        if 'imageinfo' in p:
            urls[p['title'].removeprefix('File:').replace(' ','_')]=p['imageinfo'][0]['url']
    print(f'batch {i//50+1}: {len(urls)} resolved', flush=True)
json.dump(urls, open('file_urls.json','w'))
print('resolved', len(urls), 'of', len(files))
missing=[f for f in files if f not in urls]
print('unresolved:', missing[:10])
