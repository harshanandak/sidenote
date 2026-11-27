function Read-HandoffTask($registered, [switch]$Staged) {
    $security = Read-ListenerSecurity $registered 7
    $definition = $registered.Definition
    if ($definition.Actions.Count -ne 1 -or $definition.Actions.Item(1).Type -ne 0) { throw 'unmanaged_action' }
    $action = $definition.Actions.Item(1)
    $task = Read-ListenerTask $action.Path -Staged:$Staged
    if ($action.Arguments -cne $env:SIDENOTE_TASK_ARGS -or
        [string]$task.Actions[0].WorkingDirectory -cne [string]$action.WorkingDirectory -or
        [bool]$task.Settings.Enabled -ne [bool]$definition.Settings.Enabled) { throw 'unmanaged_action' }
    @{
        exe = $action.Path; args = $action.Arguments; working_dir = [string]$action.WorkingDirectory
        xml = $definition.XmlText; security = $security; enabled = [bool]$definition.Settings.Enabled
        running = ($registered.State -eq 4)
    }
}

function Invoke-ListenerHandoff($registered) {
    $path = $env:SIDENOTE_HANDOFF_REQUEST
    $request = [IO.File]::ReadAllText($path) | ConvertFrom-Json
    if (!$registered) {
        if ($request.expected) { throw 'task_changed' }
        return
    }
    $before = Read-HandoffTask $registered -Staged:($null -ne $request.expected -or $request.retry)
    if ($request.expected) {
        if ($before.xml -cne $request.expected -or $before.security -cne $request.security) { throw 'task_changed' }
        $definition = $service.NewTask(0)
        $definition.XmlText = $request.replacement
        if ($definition.Actions.Item(1).Path -ceq $before.exe -and $definition.Actions.Item(1).WorkingDirectory -ceq $before.working_dir) {
            $registered.Enabled = $definition.Settings.Enabled
        } else {
            # Update only, retain the ACL, and suppress registration triggers.
            $null = $folder.RegisterTask($env:SIDENOTE_TASK_NAME, $request.replacement, 52, $null, $null, 3, $null)
        }
        $after = Read-HandoffTask (Open-ListenerTask) -Staged
        if ($after.xml -cne $request.replacement -or $after.security -cne $request.security) { throw 'task_changed' }
        return
    }

    $definition = $service.NewTask(0)
    $definition.XmlText = $before.xml
    $definition.Settings.Enabled = $false
    $disabled = $definition.XmlText
    $action = $definition.Actions.Item(1)
    $action.Path = $env:SIDENOTE_TASK_EXE
    if ($action.WorkingDirectory) { $action.WorkingDirectory = [IO.Path]::GetDirectoryName($action.Path) }
    $switched = $definition.XmlText
    $definition.Settings.Enabled = $true
    [IO.File]::WriteAllText($path, (@{
        prior = $before; disabled = $disabled; switched = $switched; enabled = $definition.XmlText
    } | ConvertTo-Json -Compress -Depth 4))
}
