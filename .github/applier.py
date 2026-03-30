import sys
import json
import yaml
import urllib.request

base = sys.argv[1].rstrip('/')
files = [
    '00_namespace.yaml',
    '01_shipclass.yaml',
    '02_storageclass.yaml',
    '03_clusternetworkclass.yaml',
    '04_networkclass.yaml',
    '05_configmap.yaml',
    '06_secret.yaml',
    '07_pvc_fs.yaml',
    '08_ship.yaml'
]
CLUSTER = {
    'Namespace': 'namespaces',
    'ShipClass': 'shipclasses',
    'StorageClass': 'storageclasses',
    'ClusterNetworkClass': 'clusternetworkclasses',
    'PersistentVolume': 'persistentvolumes'
}
NAMESP = {
    'Ship': 'ships',
    'NetworkClass': 'networkclasses',
    'ConfigMap': 'configmaps',
    'Secret': 'secrets',
    'PersistentVolumeClaim': 'persistentvolumeclaims',
    'Lease': 'leases'
}
for fn in files:
    path = sys.argv[2] + '/' + fn
    with open(path, 'r') as f:
        data = yaml.safe_load(f)
    payload = json.dumps(data).encode('utf-8')
    # resolve URL (subset of apply.sh logic)
    kind = data.get('kind', '')
    namespace = data.get('metadata', {}).get('namespace', '')
    if kind in CLUSTER:
        url = f"{base}/api/v1/{CLUSTER[kind]}"
    elif kind in NAMESP:
        if not namespace:
            print('ERROR: missing namespace for', kind)
            sys.exit(1)
        url = f"{base}/api/v1/namespaces/{namespace}/{NAMESP[kind]}"
    else:
        print('Skipping unknown kind', kind)
        continue
    req = urllib.request.Request(url, data=payload, headers={'Content-Type': 'application/json'}, method='POST')
    try:
        with urllib.request.urlopen(req, timeout=30) as r:
            print(fn, '→', r.status)
    except Exception as e:
        print(fn, 'failed:', e)
