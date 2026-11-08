#include <windows.h>
#include <shobjidl.h>
#include <propkey.h>
#include <propvarutil.h>
#include <wrl/client.h>
#include <cwchar>

using Microsoft::WRL::ComPtr;

// Called from an initialized COM apartment. All string storage remains caller-owned.
static HRESULT put(IPropertyStore* store, REFPROPERTYKEY key, const wchar_t* text) {
    PROPVARIANT value{};
    HRESULT hr = InitPropVariantFromString(text, &value);
    if (SUCCEEDED(hr)) hr = store->SetValue(key, value);
    PropVariantClear(&value);
    return hr;
}

struct Identity {
    DWORD pid;
    const wchar_t *id, *name, *command, *icon;
    unsigned tagged;
};

static BOOL CALLBACK visit(HWND hwnd, LPARAM param) {
    auto& profile = *reinterpret_cast<Identity*>(param);
    DWORD pid = 0;
    GetWindowThreadProcessId(hwnd, &pid);
    if (pid != profile.pid || GetWindow(hwnd, GW_OWNER) ||
        (GetWindowLongPtrW(hwnd, GWL_EXSTYLE) & WS_EX_TOOLWINDOW)) return TRUE;
    wchar_t cls[64]{};
    if (!GetClassNameW(hwnd, cls, 64) || wcscmp(cls, L"Chrome_WidgetWin_1")) return TRUE;

    ComPtr<IPropertyStore> store;
    if (FAILED(SHGetPropertyStoreForWindow(hwnd, IID_PPV_ARGS(&store)))) return TRUE;
    PROPVARIANT old{};
    store->GetValue(PKEY_AppUserModel_ID, &old);
    bool same = old.vt == VT_LPWSTR && old.pwszVal && !wcscmp(old.pwszVal, profile.id);
    PropVariantClear(&old);
    PROPVARIANT oldIcon{};
    store->GetValue(PKEY_AppUserModel_RelaunchIconResource, &oldIcon);
    same = same && oldIcon.vt == VT_LPWSTR && oldIcon.pwszVal && !wcscmp(oldIcon.pwszVal, profile.icon);
    PropVariantClear(&oldIcon);
    if (!same) {
        if (FAILED(put(store.Get(), PKEY_AppUserModel_RelaunchCommand, profile.command)) ||
            FAILED(put(store.Get(), PKEY_AppUserModel_RelaunchDisplayNameResource, profile.name)) ||
            FAILED(put(store.Get(), PKEY_AppUserModel_RelaunchIconResource, profile.icon)) ||
            FAILED(put(store.Get(), PKEY_AppUserModel_ID, profile.id)) || FAILED(store->Commit())) return TRUE;
        if (IsWindowVisible(hwnd)) {
            ComPtr<ITaskbarList> bar;
            if (SUCCEEDED(CoCreateInstance(CLSID_TaskbarList, nullptr, CLSCTX_INPROC_SERVER, IID_PPV_ARGS(&bar))) &&
                SUCCEEDED(bar->HrInit()) && SUCCEEDED(bar->DeleteTab(hwnd))) bar->AddTab(hwnd);
        }
    }
    if (IsWindowVisible(hwnd) && (!same || !GetPropW(hwnd, L"ClaudeHydra.OverlaySet"))) {
        ComPtr<ITaskbarList3> bar;
        if (SUCCEEDED(CoCreateInstance(CLSID_TaskbarList, nullptr, CLSCTX_INPROC_SERVER, IID_PPV_ARGS(&bar))) &&
            SUCCEEDED(bar->HrInit())) {
            auto icon = static_cast<HICON>(LoadImageW(nullptr, profile.icon, IMAGE_ICON, 32, 32, LR_LOADFROMFILE));
            if (icon) {
                if (SUCCEEDED(bar->SetOverlayIcon(hwnd, icon, profile.name)))
                    SetPropW(hwnd, L"ClaudeHydra.OverlaySet", reinterpret_cast<HANDLE>(1));
                DestroyIcon(icon);
            }
        }
    }
    profile.tagged++;
    return TRUE;
}

extern "C" unsigned hydra_tag_windows(DWORD pid, const wchar_t* id, const wchar_t* name,
                                       const wchar_t* command, const wchar_t* icon) {
    HRESULT init = CoInitializeEx(nullptr, COINIT_APARTMENTTHREADED);
    if (FAILED(init) && init != RPC_E_CHANGED_MODE) return 0;
    Identity profile{pid, id, name, command, icon, 0};
    EnumWindows(visit, reinterpret_cast<LPARAM>(&profile));
    if (SUCCEEDED(init)) CoUninitialize();
    return profile.tagged;
}

extern "C" HRESULT hydra_tag_shortcut(const wchar_t* path, const wchar_t* id, const wchar_t* icon) {
    HRESULT init = CoInitializeEx(nullptr, COINIT_APARTMENTTHREADED);
    if (FAILED(init) && init != RPC_E_CHANGED_MODE) return init;
    HRESULT result = [&]() {
        ComPtr<IShellLinkW> link;
        ComPtr<IPersistFile> file;
        ComPtr<IPropertyStore> store;
        HRESULT hr = CoCreateInstance(CLSID_ShellLink, nullptr, CLSCTX_INPROC_SERVER, IID_PPV_ARGS(&link));
        if (SUCCEEDED(hr)) hr = link.As(&file);
        if (SUCCEEDED(hr)) hr = file->Load(path, STGM_READWRITE);
        if (SUCCEEDED(hr)) hr = link->SetIconLocation(icon, 0);
        if (SUCCEEDED(hr)) hr = link.As(&store);
        if (SUCCEEDED(hr)) hr = put(store.Get(), PKEY_AppUserModel_ID, id);
        if (SUCCEEDED(hr)) hr = store->Commit();
        if (SUCCEEDED(hr)) hr = file->Save(path, TRUE);
        return hr;
    }();
    if (SUCCEEDED(init)) CoUninitialize();
    return result;
}
