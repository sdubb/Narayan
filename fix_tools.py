import re, glob, os
os.chdir('/home/shubham/Narayan')
files = [f for f in glob.glob('src/tools/*.rs') if not f.endswith('mod.rs')]
for fpath in files:
    with open(fpath, 'r') as fh:
        content = fh.read()
    orig = content

    # 1. Fix output_schema return type
    content = content.replace(
        'fn output_schema(&self) -> serde_json::Value {',
        'fn output_schema(&self) -> Option<serde_json::Value> {')

    # 2. Remove required_resources blocks
    content = re.sub(
        r'\n\s*fn required_resources\(&self\) -> Vec<crate::tools::ResourceDescriptor> \{\s*\n\s*vec!\[\]\s*\n\s*\}',
        '', content)

    # 3. Remove capability_constraints blocks
    content = re.sub(
        r'\n\s*fn capability_constraints\(&self\) -> Vec<String> \{\s*\n\s*vec!\["requires_tooling"\.to_string\(\)\]\s*\n\s*\}',
        '', content)

    # 4. Wrap serde_json::json! body in Some() inside output_schema
    lines = content.split('\n')
    new_lines = []
    in_output_schema = False
    schema_started = False
    for i, line in enumerate(lines):
        if 'fn output_schema(&self) -> Option<serde_json::Value>' in line:
            in_output_schema = True
            schema_started = False
            new_lines.append(line)
            continue
        if in_output_schema and not schema_started:
            stripped = line.strip()
            if stripped.startswith('serde_json::json!'):
                indent = len(line) - len(line.lstrip())
                new_lines.append(' ' * indent + 'Some(' + stripped)
                schema_started = True
                continue
            elif stripped.startswith('Some('):
                # Already wrapped
                new_lines.append(line)
                schema_started = True
                in_output_schema = False
                continue
        if in_output_schema and schema_started:
            stripped = line.strip()
            if stripped == '})' or stripped == '}))':
                indent = len(line) - len(line.lstrip())
                new_lines.append(' ' * indent + '}))')
                in_output_schema = False
                schema_started = False
                continue
        new_lines.append(line)
    content = '\n'.join(new_lines)

    # 5. Add schema imports where needed
    schema_fns = []
    for fn_name in ['schema_string', 'schema_boolean', 'schema_integer', 'schema_array', 'schema_number']:
        if fn_name + '(' in content:
            already = False
            for line in content.split('\n')[:25]:
                if 'use crate::tools' in line and fn_name in line:
                    already = True
                    break
            if not already:
                schema_fns.append(fn_name)
    if schema_fns:
        m = re.search(r'use crate::tools::\{([^}]+)\};', content)
        if m:
            existing = m.group(1)
            new_fns = [f for f in schema_fns if f not in existing]
            if new_fns:
                content = content.replace(m.group(0),
                    'use crate::tools::{' + existing + ', ' + ', '.join(new_fns) + '};')

    if content != orig:
        print(f'Fixed: {fpath}')
        with open(fpath, 'w') as fh:
            fh.write(content)

print('Done.')
