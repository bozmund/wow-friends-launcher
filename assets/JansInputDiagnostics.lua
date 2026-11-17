local frame = CreateFrame("Frame", "JansInputDiagnosticsPanel", UIParent, "BackdropTemplate")
frame:SetSize(540, 240)
frame:SetPoint("TOPLEFT", UIParent, "TOPLEFT", 12, -80)
frame:SetFrameStrata("DIALOG")
frame:EnableMouse(false)
frame:SetBackdrop({bgFile="Interface\\Buttons\\WHITE8X8", edgeFile="Interface\\Buttons\\WHITE8X8", edgeSize=1})
frame:SetBackdropColor(0, 0, 0, 0.88)
frame:SetBackdropBorderColor(0.8, 0.65, 0.3, 1)
local text = frame:CreateFontString(nil, "OVERLAY", "GameFontNormalSmall")
text:SetPoint("TOPLEFT", 9, -9)
text:SetPoint("BOTTOMRIGHT", -9, 9)
text:SetJustifyH("LEFT")
text:SetJustifyV("TOP")
local db, previousState, elapsed = nil, nil, 0
local counts = {enter=0, escape=0, send=0, preSend=0, errors=0}
local hooked = setmetatable({}, {__mode="k"})
local lastError = "none"
local liveElapsed, sequence, pendingSource = 0, 0, nil
-- Enabled only by the launcher's explicit support consent. No chat contents or commands.
local function transmit(kind, value)
    if not C_ChatInfo or not C_ChatInfo.SendAddonMessage or not UnitName then return end
    local name = UnitName("player")
    if not name or (IsLoggedIn and not IsLoggedIn()) then return end
    sequence=(sequence+1)%10000
    local payload=tostring(value):gsub("[%c]", " "):gsub("|", "/")
    local stamp=GetServerTime and GetServerTime() or 0
    local total=math.ceil(math.min(#payload,240)/16)
    for offset=1,math.min(#payload,240),16 do
        pcall(C_ChatInfo.SendAddonMessage,"JANS_DIAG",string.format("1:%x:%d:%s:%d/%d:%s",stamp,sequence,kind:sub(1,1),math.floor((offset-1)/16)+1,total,payload:sub(offset,offset+15)),"WHISPER",name)
    end
end
local function safe(fn, ...)
    if type(fn) ~= "function" then return "unavailable" end
    local ok, value = pcall(fn, ...)
    if not ok then return "unavailable" end
    if value == nil then return "nil" end
    if type(value) == "boolean" then return value and "true" or "false" end
    if type(value) == "number" or type(value) == "string" then return tostring(value) end
    return type(value)
end
local function clean(value)
    local s = tostring(value):gsub("|", "/"):gsub("[%c]", " ")
    s = s:gsub("[Bb]earer%s+[%w%._%-]+", "Bearer [redacted]")
    return s:sub(1, 800)
end
local function record(kind, value)
    if not db then return end
    local events = db.events
    events[#events+1] = {time=safe(GetTime), kind=kind, value=clean(value)}
    if #events > 120 then table.remove(events, 1) end
end
local originalErrorHandler = geterrorhandler()
seterrorhandler(function(message)
    counts.errors = counts.errors + 1
    lastError = clean(message)
    record("lua_error", lastError)
    -- Keep free-form error text local; server receives only source location and error count.
    local source=lastError:match("([^%s]+%.lua:%d+)") or "unknown-source"
    pendingSource=source.." count="..counts.errors
    return originalErrorHandler(message)
end)
local function frameName(f)
    if not f then return "none" end
    return safe(f.GetName, f)
end
local function watchChat()
    for i=1,10 do
        local box = _G["ChatFrame"..i.."EditBox"]
        if box and not hooked[box] then
            hooked[box] = true
            box:HookScript("OnEnterPressed", function() counts.enter=counts.enter+1; record("chat_enter", frameName(box)) end)
            box:HookScript("OnEscapePressed", function() counts.escape=counts.escape+1 end)
            if type(box.SendMessage)=="function" then
                hooksecurefunc(box, "SendMessage", function() counts.send=counts.send+1 end)
            end
        end
    end
end
if EventRegistry and EventRegistry.RegisterCallback then
    EventRegistry:RegisterCallback("ChatFrame.OnEditBoxPreSendText", function() counts.preSend=counts.preSend+1 end, frame)
end
local function update()
    watchChat()
    local focus = GetCurrentKeyBoardFocus and GetCurrentKeyBoardFocus()
    local state = table.concat({
        "Control="..safe(HasFullControl).."  connected="..safe(IsConnected).."  combat="..safe(InCombatLockdown),
        "Esc="..safe(GetBindingAction,"ESCAPE",true).."  menuLock="..tostring(DISALLOW_FRAME_TOGGLING),
        "Gamepad="..safe(GetCVar,"GamePadEnable").."  NPE="..safe(GetCVar,"showNPETutorials"),
        "Cinematic="..safe(InCinematic).."  movie="..(MovieFrame and safe(MovieFrame.IsShown,MovieFrame) or "absent"),
        "Keyboard focus="..frameName(focus),
        "Chat Enter="..counts.enter.."  preSend="..counts.preSend.."  sendOK="..counts.send.."  Esc="..counts.escape,
        "Lua errors="..counts.errors,
    }, "\n")
    if db then db.latest=state end
    if state~=previousState then record("state",state); previousState=state end
    text:SetText("Jan's temporary input diagnostics v1\n"..state.."\nLast error: "..lastError:sub(1,500))
end
frame:RegisterEvent("ADDON_LOADED")
frame:RegisterEvent("PLAYER_ENTERING_WORLD")
frame:RegisterEvent("PLAYER_CONTROL_LOST")
frame:RegisterEvent("PLAYER_CONTROL_GAINED")
frame:RegisterEvent("ADDON_ACTION_BLOCKED")
frame:RegisterEvent("ADDON_ACTION_FORBIDDEN")
frame:SetScript("OnEvent", function(_, event, name, action)
    if event=="ADDON_LOADED" and name=="JansInputDiagnostics" then
        JansInputDiagnosticsDB={version=1, events={}}
        db=JansInputDiagnosticsDB
        if C_ChatInfo and C_ChatInfo.RegisterAddonMessagePrefix then pcall(C_ChatInfo.RegisterAddonMessagePrefix,"JANS_DIAG") end
        if not JANS_DIAGNOSTICS_SHOW_PANEL then frame:Hide() end
    elseif event~="ADDON_LOADED" then
        record(event, (event=="ADDON_ACTION_BLOCKED" or event=="ADDON_ACTION_FORBIDDEN") and (clean(name).." "..clean(action)) or "observed")
    end
end)
-- A hidden panel does not receive OnUpdate. Use an independent, invisible observer.
local observer = CreateFrame("Frame")
observer:SetScript("OnUpdate", function(_, dt)
    elapsed=elapsed+dt
    if elapsed>=1 then elapsed=0; update() end
    liveElapsed=liveElapsed+dt
    if liveElapsed>=30 then
        liveElapsed=0
        transmit("state",table.concat({"ctrl="..safe(HasFullControl),"net="..safe(IsConnected),"pad="..safe(GetCVar,"GamePadEnable"),"npe="..safe(GetCVar,"showNPETutorials"),"lock="..tostring(DISALLOW_FRAME_TOGGLING),"esc="..safe(GetBindingAction,"ESCAPE",true),"enter="..counts.enter,"pre="..counts.preSend,"sent="..counts.send,"err="..counts.errors}," "))
        if pendingSource then transmit("error",pendingSource); pendingSource=nil end
    end
end)
