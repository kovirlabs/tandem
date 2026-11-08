Your model does not support native tool calls. To use a tool, put exactly one line in your reply of the form:
<tool>{"name": "<tool name>", "arguments": { ... }}</tool>
Then write your short reply to the user on the next line. Available tools (JSON schema):
{{tools_json}}
