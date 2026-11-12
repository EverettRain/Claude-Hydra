/*
 * Hydra identity guard: loaded by Sandboxie (InjectDll64) into every process of a Hydra box.
 *
 * Sandboxie virtualizes files and the registry, but the system still describes itself the same
 * way to every box: the machine GUID, the hardware profile GUID, volume serials, adapter MAC
 * addresses and the computer name a boxed program reads are this computer's. Accounts kept side
 * by side then look like one device to anything that fingerprints hardware.
 *
 * Hydra derives its own values per account (identity.rs) and stores them as settings on the box
 * (`HydraMachineGuid` and the others below). This DLL answers the matching system calls with
 * those values. Without `HydraMachineGuid` nothing is hooked, so a box that does not use the
 * option behaves exactly as before.
 *
 * Where a value has many Win32 entry points it is rewritten at the native call they all end in,
 * so no route reports the host's copy while another reports the derived one:
 *   - registry values: NtQueryValueKey / NtEnumerateValueKey (RegQueryValueExW/A, RegGetValueW/A,
 *     RegEnumValueW/A, and anything else reading the value);
 *   - volume serials: NtQueryVolumeInformationFile (GetVolumeInformationW/A, ...ByHandleW,
 *     GetFileInformationByHandle, Node's fs.stat().dev) and NtQueryInformationFile
 *     (FileIdInformation, the 64-bit serial).
 *
 * Not covered: WMI (answered by WmiPrvSE outside the box), the SMBIOS tables behind
 * GetSystemFirmwareTable and the disk serial behind IOCTL_STORAGE_QUERY_PROPERTY (both could be
 * rewritten here, but WMI would keep reporting the host's values, and two answers that disagree
 * are a stronger signal than one shared answer), the DHCPv6 DUID (same reason: `ipconfig` prints
 * the host's copy from outside the box), the user name and SIDs, and NtQueryMultipleValueKey.
 */
#include <winsock2.h>
#include <windows.h>
#include <winternl.h>
#include <ws2ipdef.h>
#include <iphlpapi.h>

#ifndef NT_SUCCESS
#define NT_SUCCESS(s) (((NTSTATUS)(s)) >= 0)
#endif
#define HYDRA_STATUS_BUFFER_OVERFLOW ((NTSTATUS)0x80000005L)
#define HYDRA_STATUS_BUFFER_TOO_SMALL ((NTSTATUS)0xC0000023L)

typedef void *(__stdcall *P_SbieDll_Hook)(const char *name, void *source, void *detour, HMODULE module);
typedef LONG(__stdcall *P_QueryProcess)(HANDLE, WCHAR *, WCHAR *, WCHAR *, ULONG *);
typedef LONG(__stdcall *P_QueryConf)(const WCHAR *, const WCHAR *, ULONG, WCHAR *, ULONG);

/* x86 decorates the stdcall export; Sandboxie looks the entry point up by its plain name. */
#ifndef _WIN64
#pragma comment(linker, "/EXPORT:InjectDllMain=_InjectDllMain@8")
#endif

typedef NTSTATUS(NTAPI *P_NtQueryValueKey)(HANDLE, PUNICODE_STRING, ULONG, PVOID, ULONG, PULONG);
typedef NTSTATUS(NTAPI *P_NtEnumerateValueKey)(HANDLE, ULONG, ULONG, PVOID, ULONG, PULONG);
typedef NTSTATUS(NTAPI *P_NtQueryKey)(HANDLE, ULONG, PVOID, ULONG, PULONG);
typedef NTSTATUS(NTAPI *P_NtQueryVolumeInformationFile)(HANDLE, PIO_STATUS_BLOCK, PVOID, ULONG, ULONG);
typedef NTSTATUS(NTAPI *P_NtQueryInformationFile)(HANDLE, PIO_STATUS_BLOCK, PVOID, ULONG, ULONG);
typedef ULONG(WINAPI *P_GetAdaptersAddresses)(ULONG, ULONG, PVOID, PIP_ADAPTER_ADDRESSES, PULONG);
typedef ULONG(WINAPI *P_GetAdaptersInfo)(PIP_ADAPTER_INFO, PULONG);
typedef DWORD(WINAPI *P_GetIfTable)(PMIB_IFTABLE, PULONG, BOOL);
typedef DWORD(WINAPI *P_GetIfEntry)(PMIB_IFROW);
typedef DWORD(WINAPI *P_GetIfTable2)(PMIB_IF_TABLE2 *);
typedef DWORD(WINAPI *P_GetIfTable2Ex)(MIB_IF_TABLE_LEVEL, PMIB_IF_TABLE2 *);
typedef DWORD(WINAPI *P_GetIfEntry2)(PMIB_IF_ROW2);
typedef DWORD(WINAPI *P_GetIfEntry2Ex)(MIB_IF_ENTRY_LEVEL, PMIB_IF_ROW2);
typedef BOOL(WINAPI *P_GetComputerNameExW)(COMPUTER_NAME_FORMAT, LPWSTR, LPDWORD);
typedef BOOL(WINAPI *P_GetComputerNameExA)(COMPUTER_NAME_FORMAT, LPSTR, LPDWORD);
typedef BOOL(WINAPI *P_GetComputerNameW)(LPWSTR, LPDWORD);
typedef BOOL(WINAPI *P_GetComputerNameA)(LPSTR, LPDWORD);
typedef BOOL(WINAPI *P_LookupAccountSidW)(LPCWSTR, PSID, LPWSTR, LPDWORD, LPWSTR, LPDWORD, PSID_NAME_USE);
typedef BOOL(WINAPI *P_LookupAccountSidA)(LPCSTR, PSID, LPSTR, LPDWORD, LPSTR, LPDWORD, PSID_NAME_USE);
typedef BOOL(WINAPI *P_LookupAccountNameW)(LPCWSTR, LPCWSTR, PSID, LPDWORD, LPWSTR, LPDWORD, PSID_NAME_USE);
typedef BOOL(WINAPI *P_LookupAccountNameA)(LPCSTR, LPCSTR, PSID, LPDWORD, LPSTR, LPDWORD, PSID_NAME_USE);
typedef BOOLEAN(WINAPI *P_GetUserNameExW)(int, LPWSTR, PULONG);
typedef BOOLEAN(WINAPI *P_GetUserNameExA)(int, LPSTR, PULONG);

/* A function can be exported by two modules (a forwarder and its target). Both copies are
   hooked so programs bound to either reach the detour; `b` is the target's trampoline and is
   preferred, since calling the forwarder's would re-enter the detour through the target. */
#define REAL(pair) ((pair).b ? (pair).b : (pair).a)
#define PAIR(type) struct { type a; type b; }

static P_NtQueryValueKey sys_NtQueryValueKey;
static P_NtEnumerateValueKey sys_NtEnumerateValueKey;
static P_NtQueryKey sys_NtQueryKey;
static P_NtQueryVolumeInformationFile sys_NtQueryVolumeInformationFile;
static P_NtQueryInformationFile sys_NtQueryInformationFile;
static P_GetAdaptersAddresses sys_GetAdaptersAddresses;
static P_GetAdaptersInfo sys_GetAdaptersInfo;
static P_GetIfTable sys_GetIfTable;
static P_GetIfEntry sys_GetIfEntry;
static P_GetIfTable2 sys_GetIfTable2;
static P_GetIfTable2Ex sys_GetIfTable2Ex;
static P_GetIfEntry2 sys_GetIfEntry2;
static P_GetIfEntry2Ex sys_GetIfEntry2Ex;
static PAIR(P_GetComputerNameExW) sys_GetComputerNameExW;
static PAIR(P_GetComputerNameExA) sys_GetComputerNameExA;
static PAIR(P_GetComputerNameW) sys_GetComputerNameW;
static PAIR(P_GetComputerNameA) sys_GetComputerNameA;
static PAIR(P_LookupAccountSidW) sys_LookupAccountSidW;
static PAIR(P_LookupAccountSidA) sys_LookupAccountSidA;
static PAIR(P_LookupAccountNameW) sys_LookupAccountNameW;
static PAIR(P_LookupAccountNameA) sys_LookupAccountNameA;
static PAIR(P_GetUserNameExW) sys_GetUserNameExW;
static PAIR(P_GetUserNameExA) sys_GetUserNameExA;

/* What this box reports, as configured on it by sandbox.rs. An empty field is left to the host. */
static WCHAR id_machine_guid[64];
static WCHAR id_hw_profile_guid[64];
static WCHAR id_computer_name[64];
static BYTE id_mac[6];
static int have_mac;
static BYTE id_volume_key[4];
static int have_volume_serial;
/* This computer's NetBIOS name, read before any hook: account lookups report it as the domain
   of local accounts, and only that exact domain is swapped for the derived name. */
static WCHAR host_name[64];

/* --- settings -------------------------------------------------------------- */

static int hex_digit(WCHAR c) {
    if (c >= L'0' && c <= L'9') return c - L'0';
    if (c >= L'a' && c <= L'f') return c - L'a' + 10;
    if (c >= L'A' && c <= L'F') return c - L'A' + 10;
    return -1;
}

/* Parses exactly `count` bytes of hex digits and nothing after them. */
static int parse_hex(const WCHAR *text, BYTE *out, int count) {
    int i;
    for (i = 0; i < count; i++) {
        int hi = hex_digit(text[i * 2]), lo;
        if (hi < 0) return 0;
        lo = hex_digit(text[i * 2 + 1]);
        if (lo < 0) return 0;
        out[i] = (BYTE)(hi * 16 + lo);
    }
    return text[count * 2] == 0;
}

static int conf_text(P_QueryConf query_conf, const WCHAR *box, const WCHAR *name, WCHAR *value, DWORD size) {
    value[0] = 0;
    if (!query_conf || query_conf(box, name, 0, value, size) != 0) return 0;
    return value[0] != 0;
}

/* Reads this box's identifiers; without a machine GUID the option is off and nothing is hooked. */
static int load_identity(HMODULE sbie) {
    WCHAR own_box[34], image[96], sid[96], hex[64];
    ULONG session = 0;
    P_QueryProcess query_process = (P_QueryProcess)(void *)GetProcAddress(sbie, "SbieApi_QueryProcess");
    P_QueryConf query_conf = (P_QueryConf)(void *)GetProcAddress(sbie, "SbieApi_QueryConf");
    if (!query_process || query_process((HANDLE)(ULONG_PTR)GetCurrentProcessId(), own_box, image, sid, &session) != 0)
        return 0;
    if (!conf_text(query_conf, own_box, L"HydraMachineGuid", id_machine_guid, sizeof(id_machine_guid))) return 0;
    if (!conf_text(query_conf, own_box, L"HydraHwProfileGuid", id_hw_profile_guid, sizeof(id_hw_profile_guid)))
        id_hw_profile_guid[0] = 0;
    if (!conf_text(query_conf, own_box, L"HydraComputerName", id_computer_name, sizeof(id_computer_name)))
        id_computer_name[0] = 0;
    have_mac = conf_text(query_conf, own_box, L"HydraMac", hex, sizeof(hex)) && parse_hex(hex, id_mac, 6);
    have_volume_serial =
        conf_text(query_conf, own_box, L"HydraVolumeSerial", hex, sizeof(hex)) && parse_hex(hex, id_volume_key, 4);
    return 1;
}

/* --- helpers --------------------------------------------------------------- */

static void hook(P_SbieDll_Hook sbie_hook, HMODULE module, HMODULE self, const char *name, void *detour, void **sys) {
    void *source = module ? (void *)GetProcAddress(module, name) : NULL;
    if (source) *sys = sbie_hook(name, source, detour, self);
}

/* Hooks `name` in both modules, but the same code only once: when one export forwards to the
   other, GetProcAddress resolves both to one address and a second hook would stack on the first. */
static void hook_pair(P_SbieDll_Hook sbie_hook, HMODULE first, HMODULE second, HMODULE self, const char *name,
                      void *detour, void **sys_a, void **sys_b) {
    void *a = first ? (void *)GetProcAddress(first, name) : NULL;
    void *b = second ? (void *)GetProcAddress(second, name) : NULL;
    if (a) *sys_a = sbie_hook(name, a, detour, self);
    if (b && b != a) *sys_b = sbie_hook(name, b, detour, self);
}

static WCHAR upper_w(WCHAR c) { return (c >= L'a' && c <= L'z') ? (WCHAR)(c - 32) : c; }

/* Case-insensitive (ASCII) equality of a counted string with a terminated one. */
static int same_text(const WCHAR *text, int chars, const WCHAR *other) {
    int i;
    for (i = 0; i < chars; i++) {
        if (!other[i] || upper_w(text[i]) != upper_w(other[i])) return 0;
    }
    return other[chars] == 0;
}

/* Case-insensitive comparison of the last `chars` characters of `text` with `suffix`. */
static int tail_is(const WCHAR *text, int chars, const WCHAR *suffix) {
    int length = (int)lstrlenW(suffix), i;
    if (chars < length) return 0;
    for (i = 0; i < length; i++) {
        if (upper_w(text[chars - length + i]) != upper_w(suffix[i])) return 0;
    }
    return 1;
}

/* The identity value kept under the open `key`, if it is one of the keys Hydra answers for:
   `*value` gets the value's name and the result is the text to report. NtQueryKey names the key
   as the system sees it, so paths through Wow6432Node, ControlSet001 or the box's own copy all
   end the same way. */
static const WCHAR *identity_key(HANDLE key, const WCHAR **value) {
    struct {
        ULONG name_length; /* KEY_NAME_INFORMATION: the length comes first, then the name. */
        WCHAR name[300];
    } info;
    ULONG length = 0;
    int chars;
    if (!sys_NtQueryKey || !key || (ULONG_PTR)key >= 0x80000000) return NULL;
    info.name_length = 0;
    if (sys_NtQueryKey(key, 3 /* KeyNameInformation */, &info, sizeof(info), &length) != 0) return NULL;
    if (info.name_length == 0 || info.name_length >= sizeof(info.name)) return NULL;
    chars = (int)(info.name_length / sizeof(WCHAR));
    if (id_machine_guid[0] && tail_is(info.name, chars, L"\\Microsoft\\Cryptography")) {
        *value = L"MachineGuid";
        return id_machine_guid;
    }
    if (id_hw_profile_guid[0] && tail_is(info.name, chars, L"\\Hardware Profiles\\0001")) {
        *value = L"HwProfileGuid";
        return id_hw_profile_guid;
    }
    /* hostname.exe prints this value instead of calling GetComputerName, so it needs answering too. */
    if (id_computer_name[0] && tail_is(info.name, chars, L"\\Services\\Tcpip\\Parameters")) {
        *value = L"Hostname";
        return id_computer_name;
    }
    return NULL;
}

static int is_identity_name(const WCHAR *name, int chars) {
    return same_text(name, chars, L"MachineGuid") || same_text(name, chars, L"HwProfileGuid") ||
           same_text(name, chars, L"Hostname");
}

/* Writes a REG_SZ value in the layout of `info_class`, with the status and length the kernel
   gives for that buffer size: too small for the fixed part, too small for the whole answer
   (fixed part written), or complete. */
static NTSTATUS answer_value(ULONG info_class, const WCHAR *name, ULONG name_bytes, const WCHAR *text, PVOID info,
                             ULONG length, PULONG result) {
    ULONG data_bytes = ((ULONG)lstrlenW(text) + 1) * sizeof(WCHAR);
    BYTE *out = (BYTE *)info;
    ULONG fixed, data_at, need;
    switch (info_class) {
    case 2: /* KeyValuePartialInformation: TitleIndex, Type, DataLength, Data */
    case 4: /* KeyValuePartialInformationAlign64: Type, DataLength, Data */
        fixed = info_class == 2 ? 12 : 8;
        need = fixed + data_bytes;
        if (result) *result = need;
        if (length < fixed) return HYDRA_STATUS_BUFFER_TOO_SMALL;
        if (info_class == 2) {
            ((ULONG *)out)[0] = 0;
            ((ULONG *)out)[1] = REG_SZ;
            ((ULONG *)out)[2] = data_bytes;
        } else {
            ((ULONG *)out)[0] = REG_SZ;
            ((ULONG *)out)[1] = data_bytes;
        }
        if (length < need) return HYDRA_STATUS_BUFFER_OVERFLOW;
        memcpy(out + fixed, text, data_bytes);
        return 0;
    case 1: /* KeyValueFullInformation: TitleIndex, Type, DataOffset, DataLength, NameLength, Name */
    case 3: /* KeyValueFullInformationAlign64 */
        fixed = 20;
        data_at = (fixed + name_bytes + 7) & ~7u;
        need = data_at + data_bytes;
        if (result) *result = need;
        if (length < fixed) return HYDRA_STATUS_BUFFER_TOO_SMALL;
        ((ULONG *)out)[0] = 0;
        ((ULONG *)out)[1] = REG_SZ;
        ((ULONG *)out)[2] = data_at;
        ((ULONG *)out)[3] = data_bytes;
        ((ULONG *)out)[4] = name_bytes;
        if (length < need) return HYDRA_STATUS_BUFFER_OVERFLOW;
        memmove(out + fixed, name, name_bytes);
        memcpy(out + data_at, text, data_bytes);
        return 0;
    }
    return -1;
}

/* FNV-1a over `key || tag || data`, finished with the murmur3 avalanche. Must match
   `identity.rs::derive` byte for byte; keyed so that relations between two derived values (the
   XOR of two volumes' serials, two adapters' addresses) differ between accounts. */
static DWORD derive(const BYTE *key, int key_len, BYTE tag, const BYTE *data, int len) {
    DWORD hash = 2166136261u;
    int i;
    for (i = 0; i < key_len; i++) hash = (hash ^ key[i]) * 16777619u;
    hash = (hash ^ tag) * 16777619u;
    for (i = 0; i < len; i++) hash = (hash ^ data[i]) * 16777619u;
    hash ^= hash >> 16;
    hash *= 0x85ebca6bu;
    hash ^= hash >> 13;
    hash *= 0xc2b2ae35u;
    return hash ^ (hash >> 16);
}

/* The serial this box reports for the volume whose real serial is `host`. Keyed on the real
   serial rather than on a path: at this level only the volume is known, and every route to one
   volume (drive letter, NULL root, mount point, file handle) then agrees. */
static DWORD volume_serial(DWORD host) {
    BYTE data[4];
    data[0] = (BYTE)(host >> 24);
    data[1] = (BYTE)(host >> 16);
    data[2] = (BYTE)(host >> 8);
    data[3] = (BYTE)host;
    return derive(id_volume_key, 4, 'V', data, 4);
}

static ULONGLONG volume_serial64(ULONGLONG host) {
    BYTE data[8];
    int i;
    for (i = 0; i < 8; i++) data[i] = (BYTE)(host >> (56 - i * 8));
    /* The low half is the 32-bit serial `vol` prints, as on NTFS; the high half is keyed apart. */
    return ((ULONGLONG)derive(id_volume_key, 4, 'H', data, 8) << 32) | volume_serial((DWORD)host);
}

/* An adapter's address from its GUID name (`{...}`): the account's three-byte prefix plus three
   bytes keyed on the adapter itself. The same adapter then has the same address through every
   API and across reboots, whatever order adapters are listed in or whether a VPN came and went. */
static void adapter_mac(const char *name, BYTE out[6]) {
    BYTE text[64];
    int length = 0;
    DWORD mix;
    if (name) {
        while (*name && *name != '{') name++;
        for (; name[0] && length < (int)sizeof(text); name++) {
            char c = name[0];
            text[length++] = (BYTE)((c >= 'a' && c <= 'z') ? c - 32 : c);
            if (c == '}') break;
        }
    }
    mix = derive(id_mac, 6, 'M', text, length);
    out[0] = id_mac[0];
    out[1] = id_mac[1];
    out[2] = id_mac[2];
    out[3] = (BYTE)(mix >> 24);
    out[4] = (BYTE)(mix >> 16);
    out[5] = (BYTE)(mix >> 8);
}

static void guid_name(const GUID *id, char out[40]) {
    static const char digits[] = "0123456789ABCDEF";
    BYTE bytes[16];
    int i, at = 0;
    bytes[0] = (BYTE)(id->Data1 >> 24);
    bytes[1] = (BYTE)(id->Data1 >> 16);
    bytes[2] = (BYTE)(id->Data1 >> 8);
    bytes[3] = (BYTE)id->Data1;
    bytes[4] = (BYTE)(id->Data2 >> 8);
    bytes[5] = (BYTE)id->Data2;
    bytes[6] = (BYTE)(id->Data3 >> 8);
    bytes[7] = (BYTE)id->Data3;
    for (i = 0; i < 8; i++) bytes[8 + i] = id->Data4[i];
    out[at++] = '{';
    for (i = 0; i < 16; i++) {
        if (i == 4 || i == 6 || i == 8 || i == 10) out[at++] = '-';
        out[at++] = digits[bytes[i] >> 4];
        out[at++] = digits[bytes[i] & 15];
    }
    out[at++] = '}';
    out[at] = 0;
}

static void wide_guid_name(const WCHAR *text, char out[40]) {
    int i = 0;
    while (*text && *text != L'{') text++;
    for (; text[0] && i < 39; text++) {
        out[i++] = (char)text[0];
        if (text[0] == L'}') break;
    }
    out[i] = 0;
}

static void rewrite_row2(PMIB_IF_ROW2 row) {
    char name[40];
    if (row->PhysicalAddressLength != 6) return;
    guid_name(&row->InterfaceGuid, name);
    adapter_mac(name, row->PhysicalAddress);
    if (row->PermanentPhysicalAddress[0] | row->PermanentPhysicalAddress[1] | row->PermanentPhysicalAddress[2] |
        row->PermanentPhysicalAddress[3] | row->PermanentPhysicalAddress[4] | row->PermanentPhysicalAddress[5])
        memcpy(row->PermanentPhysicalAddress, row->PhysicalAddress, 6);
}

static void rewrite_row(PMIB_IFROW row) {
    char name[40];
    if (row->dwPhysAddrLen != 6) return;
    wide_guid_name(row->wszName, name);
    adapter_mac(name, row->bPhysAddr);
}

static BOOL answer_name_w(const WCHAR *name, LPWSTR buffer, LPDWORD size) {
    DWORD need = (DWORD)lstrlenW(name) + 1;
    if (!size) {
        SetLastError(ERROR_INVALID_PARAMETER);
        return FALSE;
    }
    if (!buffer || *size < need) {
        *size = need;
        SetLastError(ERROR_MORE_DATA);
        return FALSE;
    }
    memcpy(buffer, name, need * sizeof(WCHAR));
    *size = need - 1;
    return TRUE;
}

static BOOL answer_name_a(const WCHAR *name, LPSTR buffer, LPDWORD size) {
    DWORD length = (DWORD)lstrlenW(name), need = length + 1, i;
    if (!size) {
        SetLastError(ERROR_INVALID_PARAMETER);
        return FALSE;
    }
    if (!buffer || *size < need) {
        *size = need;
        SetLastError(ERROR_MORE_DATA);
        return FALSE;
    }
    /* The derived names are ASCII, so narrowing cannot lose anything. */
    for (i = 0; i < length; i++) buffer[i] = (CHAR)name[i];
    buffer[length] = 0;
    *size = length;
    return TRUE;
}

static int is_host_w(const WCHAR *text, int chars) { return host_name[0] && same_text(text, chars, host_name); }

static int is_host_a(const char *text, int chars) {
    int i;
    if (!host_name[0]) return 0;
    for (i = 0; i < chars; i++) {
        if (!host_name[i] || upper_w((WCHAR)(BYTE)text[i]) != upper_w(host_name[i])) return 0;
    }
    return host_name[chars] == 0;
}

/* After a successful account lookup: a local account's domain is the computer name, which is
   swapped for the derived one. `capacity` is what the caller offered; if the derived name does
   not fit, the call fails the way the system's does, asking for the size it needs. */
static BOOL swap_domain_w(LPWSTR domain, LPDWORD domain_size, DWORD capacity) {
    DWORD length = (DWORD)lstrlenW(id_computer_name);
    if (!domain || !domain_size || !is_host_w(domain, (int)*domain_size)) return TRUE;
    if (capacity < length + 1) {
        *domain_size = length + 1;
        SetLastError(ERROR_INSUFFICIENT_BUFFER);
        return FALSE;
    }
    memcpy(domain, id_computer_name, (length + 1) * sizeof(WCHAR));
    *domain_size = length;
    return TRUE;
}

static BOOL swap_domain_a(LPSTR domain, LPDWORD domain_size, DWORD capacity) {
    DWORD length = (DWORD)lstrlenW(id_computer_name), i;
    if (!domain || !domain_size || !is_host_a(domain, (int)*domain_size)) return TRUE;
    if (capacity < length + 1) {
        *domain_size = length + 1;
        SetLastError(ERROR_INSUFFICIENT_BUFFER);
        return FALSE;
    }
    for (i = 0; i <= length; i++) domain[i] = (CHAR)id_computer_name[i];
    *domain_size = length;
    return TRUE;
}

/* A size probe asks for the host's length; the derived name may be longer. */
static void grow_domain_size(LPDWORD domain_size) {
    DWORD need = (DWORD)lstrlenW(id_computer_name) + 1;
    if (domain_size && GetLastError() == ERROR_INSUFFICIENT_BUFFER && *domain_size < need) *domain_size = need;
}

/* --- detours: registry ------------------------------------------------------ */

static NTSTATUS NTAPI my_NtQueryValueKey(HANDLE key, PUNICODE_STRING name, ULONG info_class, PVOID info,
                                         ULONG length, PULONG result) {
    const WCHAR *value = NULL, *text;
    /* The value name is compared first: it is cheap, and almost every query is for another value. */
    if (info_class >= 1 && info_class <= 4 && name && name->Buffer &&
        is_identity_name(name->Buffer, name->Length / sizeof(WCHAR))) {
        text = identity_key(key, &value);
        if (text && same_text(name->Buffer, name->Length / sizeof(WCHAR), value))
            return answer_value(info_class, name->Buffer, name->Length, text, info, length, result);
    }
    return sys_NtQueryValueKey(key, name, info_class, info, length, result);
}

/* `reg query <key>` without /v lists every value by index; the identity value must not come
   through with the host's data that way. */
static NTSTATUS NTAPI my_NtEnumerateValueKey(HANDLE key, ULONG index, ULONG info_class, PVOID info, ULONG length,
                                             PULONG result) {
    NTSTATUS status = sys_NtEnumerateValueKey(key, index, info_class, info, length, result);
    const WCHAR *value = NULL, *text;
    struct {
        ULONG title_index, type, name_length; /* KEY_VALUE_BASIC_INFORMATION */
        WCHAR name[64];
    } basic;
    ULONG basic_length = 0;
    if (info_class < 1 || info_class > 4) return status;
    if (!NT_SUCCESS(status) && status != HYDRA_STATUS_BUFFER_OVERFLOW && status != HYDRA_STATUS_BUFFER_TOO_SMALL)
        return status;
    text = identity_key(key, &value);
    if (!text) return status;
    /* The name by index, from the basic class: the partial classes carry no name at all. */
    if (sys_NtEnumerateValueKey(key, index, 0, &basic, sizeof(basic), &basic_length) != 0) return status;
    if (!same_text(basic.name, (int)(basic.name_length / sizeof(WCHAR)), value)) return status;
    return answer_value(info_class, basic.name, basic.name_length, text, info, length, result);
}

/* --- detours: volumes ------------------------------------------------------- */

static NTSTATUS NTAPI my_NtQueryVolumeInformationFile(HANDLE file, PIO_STATUS_BLOCK io, PVOID info, ULONG length,
                                                      ULONG info_class) {
    NTSTATUS status = sys_NtQueryVolumeInformationFile(file, io, info, length, info_class);
    /* FileFsVolumeInformation: VolumeCreationTime (8 bytes), then VolumeSerialNumber. A label
       that does not fit still leaves the fixed part filled in. */
    if (info_class == 1 && info && length >= 12 && (NT_SUCCESS(status) || status == HYDRA_STATUS_BUFFER_OVERFLOW)) {
        DWORD *serial = (DWORD *)((BYTE *)info + 8);
        *serial = volume_serial(*serial);
    }
    return status;
}

static NTSTATUS NTAPI my_NtQueryInformationFile(HANDLE file, PIO_STATUS_BLOCK io, PVOID info, ULONG length,
                                                ULONG info_class) {
    NTSTATUS status = sys_NtQueryInformationFile(file, io, info, length, info_class);
    /* FileIdInformation: the 64-bit VolumeSerialNumber comes first. */
    if (info_class == 59 && info && length >= 8 && NT_SUCCESS(status)) {
        ULONGLONG *serial = (ULONGLONG *)info;
        *serial = volume_serial64(*serial);
    }
    return status;
}

/* --- detours: adapters ------------------------------------------------------ */

static ULONG WINAPI my_GetAdaptersAddresses(ULONG family, ULONG flags, PVOID reserved,
                                            PIP_ADAPTER_ADDRESSES addresses, PULONG size) {
    ULONG result = sys_GetAdaptersAddresses(family, flags, reserved, addresses, size);
    PIP_ADAPTER_ADDRESSES adapter;
    if (result != NO_ERROR || !addresses) return result;
    for (adapter = addresses; adapter; adapter = adapter->Next) {
        if (adapter->PhysicalAddressLength != 6) continue;
        adapter_mac(adapter->AdapterName, adapter->PhysicalAddress);
    }
    return result;
}

static ULONG WINAPI my_GetAdaptersInfo(PIP_ADAPTER_INFO adapters, PULONG size) {
    ULONG result = sys_GetAdaptersInfo(adapters, size);
    PIP_ADAPTER_INFO adapter;
    if (result != NO_ERROR || !adapters) return result;
    for (adapter = adapters; adapter; adapter = adapter->Next) {
        if (adapter->AddressLength != 6) continue;
        adapter_mac(adapter->AdapterName, adapter->Address);
    }
    return result;
}

static DWORD WINAPI my_GetIfTable(PMIB_IFTABLE table, PULONG size, BOOL order) {
    DWORD result = sys_GetIfTable(table, size, order), i;
    if (result == NO_ERROR && table)
        for (i = 0; i < table->dwNumEntries; i++) rewrite_row(&table->table[i]);
    return result;
}

static DWORD WINAPI my_GetIfEntry(PMIB_IFROW row) {
    DWORD result = sys_GetIfEntry(row);
    if (result == NO_ERROR && row) rewrite_row(row);
    return result;
}

static void rewrite_table2(PMIB_IF_TABLE2 table) {
    ULONG i;
    if (table)
        for (i = 0; i < table->NumEntries; i++) rewrite_row2(&table->Table[i]);
}

static DWORD WINAPI my_GetIfTable2(PMIB_IF_TABLE2 *table) {
    DWORD result = sys_GetIfTable2(table);
    if (result == NO_ERROR && table) rewrite_table2(*table);
    return result;
}

static DWORD WINAPI my_GetIfTable2Ex(MIB_IF_TABLE_LEVEL level, PMIB_IF_TABLE2 *table) {
    DWORD result = sys_GetIfTable2Ex(level, table);
    if (result == NO_ERROR && table) rewrite_table2(*table);
    return result;
}

static DWORD WINAPI my_GetIfEntry2(PMIB_IF_ROW2 row) {
    DWORD result = sys_GetIfEntry2(row);
    if (result == NO_ERROR && row) rewrite_row2(row);
    return result;
}

static DWORD WINAPI my_GetIfEntry2Ex(MIB_IF_ENTRY_LEVEL level, PMIB_IF_ROW2 row) {
    DWORD result = sys_GetIfEntry2Ex(level, row);
    if (result == NO_ERROR && row) rewrite_row2(row);
    return result;
}

/* --- detours: computer name ------------------------------------------------- */

/* Every COMPUTER_NAME_FORMAT is answered with the same name: they all describe one machine. */
static BOOL WINAPI my_GetComputerNameExW(COMPUTER_NAME_FORMAT format, LPWSTR buffer, LPDWORD size) {
    if (!id_computer_name[0]) return REAL(sys_GetComputerNameExW)(format, buffer, size);
    return answer_name_w(id_computer_name, buffer, size);
}

static BOOL WINAPI my_GetComputerNameExA(COMPUTER_NAME_FORMAT format, LPSTR buffer, LPDWORD size) {
    if (!id_computer_name[0]) return REAL(sys_GetComputerNameExA)(format, buffer, size);
    return answer_name_a(id_computer_name, buffer, size);
}

static BOOL WINAPI my_GetComputerNameW(LPWSTR buffer, LPDWORD size) {
    if (!id_computer_name[0]) return REAL(sys_GetComputerNameW)(buffer, size);
    return answer_name_w(id_computer_name, buffer, size);
}

static BOOL WINAPI my_GetComputerNameA(LPSTR buffer, LPDWORD size) {
    if (!id_computer_name[0]) return REAL(sys_GetComputerNameA)(buffer, size);
    return answer_name_a(id_computer_name, buffer, size);
}

/* --- detours: account domain ------------------------------------------------ */

/* A local account's domain is the computer name: `whoami` prints `HOST\user`, and .NET's
   UserDomainName reads it the same way. */
static BOOL WINAPI my_LookupAccountSidW(LPCWSTR system, PSID sid, LPWSTR name, LPDWORD name_size, LPWSTR domain,
                                       LPDWORD domain_size, PSID_NAME_USE use) {
    DWORD capacity = domain_size ? *domain_size : 0;
    BOOL ok = REAL(sys_LookupAccountSidW)(system, sid, name, name_size, domain, domain_size, use);
    if (!id_computer_name[0]) return ok;
    if (!ok) {
        grow_domain_size(domain_size);
        return ok;
    }
    return swap_domain_w(domain, domain_size, capacity);
}

static BOOL WINAPI my_LookupAccountSidA(LPCSTR system, PSID sid, LPSTR name, LPDWORD name_size, LPSTR domain,
                                       LPDWORD domain_size, PSID_NAME_USE use) {
    DWORD capacity = domain_size ? *domain_size : 0;
    BOOL ok = REAL(sys_LookupAccountSidA)(system, sid, name, name_size, domain, domain_size, use);
    if (!id_computer_name[0]) return ok;
    if (!ok) {
        grow_domain_size(domain_size);
        return ok;
    }
    return swap_domain_a(domain, domain_size, capacity);
}

static BOOL WINAPI my_LookupAccountNameW(LPCWSTR system, LPCWSTR account, PSID sid, LPDWORD sid_size, LPWSTR domain,
                                        LPDWORD domain_size, PSID_NAME_USE use) {
    DWORD capacity = domain_size ? *domain_size : 0;
    BOOL ok = REAL(sys_LookupAccountNameW)(system, account, sid, sid_size, domain, domain_size, use);
    if (!id_computer_name[0]) return ok;
    if (!ok) {
        grow_domain_size(domain_size);
        return ok;
    }
    return swap_domain_w(domain, domain_size, capacity);
}

static BOOL WINAPI my_LookupAccountNameA(LPCSTR system, LPCSTR account, PSID sid, LPDWORD sid_size, LPSTR domain,
                                        LPDWORD domain_size, PSID_NAME_USE use) {
    DWORD capacity = domain_size ? *domain_size : 0;
    BOOL ok = REAL(sys_LookupAccountNameA)(system, account, sid, sid_size, domain, domain_size, use);
    if (!id_computer_name[0]) return ok;
    if (!ok) {
        grow_domain_size(domain_size);
        return ok;
    }
    return swap_domain_a(domain, domain_size, capacity);
}

/* NameSamCompatible is `HOST\user` for a local account; the other formats do not carry the
   computer name. */
static BOOLEAN WINAPI my_GetUserNameExW(int format, LPWSTR buffer, PULONG size) {
    ULONG capacity = size ? *size : 0, host_length = (ULONG)lstrlenW(host_name), length, rest;
    BOOLEAN ok = REAL(sys_GetUserNameExW)(format, buffer, size);
    length = (ULONG)lstrlenW(id_computer_name);
    if (format != 2 || !id_computer_name[0] || !size) return ok;
    if (!ok) {
        if (GetLastError() == ERROR_MORE_DATA && length > host_length) *size += length - host_length;
        return ok;
    }
    if (!buffer || *size <= host_length || buffer[host_length] != L'\\' || !is_host_w(buffer, (int)host_length))
        return ok;
    rest = *size - host_length; /* `\user`, without the terminator */
    if (capacity < length + rest + 1) {
        *size = length + rest + 1;
        SetLastError(ERROR_MORE_DATA);
        return FALSE;
    }
    memmove(buffer + length, buffer + host_length, (rest + 1) * sizeof(WCHAR));
    memcpy(buffer, id_computer_name, length * sizeof(WCHAR));
    *size = length + rest;
    return ok;
}

static BOOLEAN WINAPI my_GetUserNameExA(int format, LPSTR buffer, PULONG size) {
    ULONG capacity = size ? *size : 0, host_length = (ULONG)lstrlenW(host_name), length, rest, i;
    BOOLEAN ok = REAL(sys_GetUserNameExA)(format, buffer, size);
    length = (ULONG)lstrlenW(id_computer_name);
    if (format != 2 || !id_computer_name[0] || !size) return ok;
    if (!ok) {
        if (GetLastError() == ERROR_MORE_DATA && length > host_length) *size += length - host_length;
        return ok;
    }
    if (!buffer || *size <= host_length || buffer[host_length] != '\\' || !is_host_a(buffer, (int)host_length))
        return ok;
    rest = *size - host_length;
    if (capacity < length + rest + 1) {
        *size = length + rest + 1;
        SetLastError(ERROR_MORE_DATA);
        return FALSE;
    }
    memmove(buffer + length, buffer + host_length, rest + 1);
    for (i = 0; i < length; i++) buffer[i] = (CHAR)id_computer_name[i];
    *size = length + rest;
    return ok;
}

/* --- entry ----------------------------------------------------------------- */

#define HOOK(module, name) hook(sbie_hook, module, self, #name, (void *)my_##name, (void **)&sys_##name)
#define HOOK_PAIR(first, second, name)                                                                          \
    hook_pair(sbie_hook, first, second, self, #name, (void *)my_##name, (void **)&sys_##name.a,                \
              (void **)&sys_##name.b)

/* Called by Sandboxie after loading this DLL, with SbieDll's module handle. */
__declspec(dllexport) void __stdcall InjectDllMain(HINSTANCE sbie, ULONG_PTR unused) {
    HMODULE ntdll = GetModuleHandleW(L"ntdll.dll");
    HMODULE kernel32 = GetModuleHandleW(L"kernel32.dll");
    /* advapi32 and kernel32 mostly forward to kernelbase / sechost, and a program whose import
       table was resolved against the target directly - the system's own reg.exe and hostname.exe
       among them - never enters the forwarding stubs, so both copies are hooked. These are not
       always loaded yet when Sandboxie injects, so they are loaded here. */
    HMODULE kernelbase = LoadLibraryW(L"kernelbase.dll");
    HMODULE advapi32 = LoadLibraryW(L"advapi32.dll");
    HMODULE sechost = LoadLibraryW(L"sechost.dll");
    HMODULE secur32, sspicli, iphlpapi;
    HMODULE self = NULL;
    DWORD host_size = (DWORD)(sizeof(host_name) / sizeof(host_name[0]));
    P_SbieDll_Hook sbie_hook = (P_SbieDll_Hook)(void *)GetProcAddress((HMODULE)sbie, "SbieDll_Hook");
    (void)unused;
    if (!sbie_hook || !ntdll || !kernel32) return;
    if (!load_identity((HMODULE)sbie)) return;
    /* The host's name, read before GetComputerNameW is hooked. */
    if (!GetComputerNameW(host_name, &host_size)) host_name[0] = 0;
    secur32 = LoadLibraryW(L"secur32.dll");
    sspicli = LoadLibraryW(L"sspicli.dll");
    iphlpapi = LoadLibraryW(L"iphlpapi.dll");
    GetModuleHandleExW(GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
                       (LPCWSTR)(void *)&InjectDllMain, &self);

    /* Not hooked, only called: the key name lookup behind the registry detours. */
    sys_NtQueryKey = (P_NtQueryKey)(void *)GetProcAddress(ntdll, "NtQueryKey");
    HOOK(ntdll, NtQueryValueKey);
    HOOK(ntdll, NtEnumerateValueKey);
    if (have_volume_serial) {
        HOOK(ntdll, NtQueryVolumeInformationFile);
        HOOK(ntdll, NtQueryInformationFile);
    }
    if (have_mac && iphlpapi) {
        HOOK(iphlpapi, GetAdaptersAddresses);
        HOOK(iphlpapi, GetAdaptersInfo);
        HOOK(iphlpapi, GetIfTable);
        HOOK(iphlpapi, GetIfEntry);
        HOOK(iphlpapi, GetIfTable2);
        HOOK(iphlpapi, GetIfTable2Ex);
        HOOK(iphlpapi, GetIfEntry2);
        HOOK(iphlpapi, GetIfEntry2Ex);
    }
    if (id_computer_name[0]) {
        HOOK_PAIR(kernel32, kernelbase, GetComputerNameExW);
        HOOK_PAIR(kernel32, kernelbase, GetComputerNameExA);
        HOOK_PAIR(kernel32, kernelbase, GetComputerNameW);
        HOOK_PAIR(kernel32, kernelbase, GetComputerNameA);
        HOOK_PAIR(advapi32, sechost, LookupAccountSidW);
        HOOK_PAIR(advapi32, sechost, LookupAccountSidA);
        HOOK_PAIR(advapi32, sechost, LookupAccountNameW);
        HOOK_PAIR(advapi32, sechost, LookupAccountNameA);
        HOOK_PAIR(secur32, sspicli, GetUserNameExW);
        HOOK_PAIR(secur32, sspicli, GetUserNameExA);
    }
}

BOOL WINAPI DllMain(HINSTANCE inst, DWORD reason, LPVOID reserved) {
    (void)reserved;
    if (reason == DLL_PROCESS_ATTACH) DisableThreadLibraryCalls(inst);
    return TRUE;
}
