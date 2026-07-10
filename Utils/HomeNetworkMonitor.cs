using System;
using System.Collections.Generic;
using System.Linq;
using System.Net;
using System.Net.NetworkInformation;
using System.Net.Sockets;
using System.Runtime.InteropServices;
using System.Threading;
using Serilog;

namespace TEAMS2HA.Utils
{
    /// <summary>
    /// Detects whether this machine is on the home network by resolving the MAC address of the
    /// default IPv4 gateway (via ARP) and comparing it against the configured home gateway MAC.
    ///
    /// The gateway MAC is used instead of SSID or broker reachability because:
    /// - it works both on Wi-Fi and wired/docked connections,
    /// - reading the SSID on Windows 11 requires location permission,
    /// - a VPN tunnel can make the broker reachable away from home, but it can never make a
    ///   foreign gateway answer ARP with the home router's MAC.
    ///
    /// An empty configured MAC disables the feature (always considered home).
    /// </summary>
    public class HomeNetworkMonitor : IDisposable
    {
        [DllImport("iphlpapi.dll", ExactSpelling = true)]
        private static extern int SendARP(uint destIp, uint srcIp, byte[] macAddr, ref uint macAddrLen);

        private readonly Func<string?> _getConfiguredMacs;
        private readonly Timer _pollTimer;
        private readonly Timer _debounceTimer;
        private readonly object _stateLock = new object();
        private bool _isHome;
        private bool _disposed;

        /// <summary>Fired when the home/away state changes. Runs on a threadpool thread.</summary>
        public event Action<bool>? HomeStateChanged;

        public bool IsHome
        {
            get { lock (_stateLock) return _isHome; }
        }

        /// <summary>True when a home gateway MAC is configured; false = feature off (always home).</summary>
        public bool IsEnabled => ParseMacs(_getConfiguredMacs()).Count > 0;

        public HomeNetworkMonitor(Func<string?> getConfiguredMacs)
        {
            _getConfiguredMacs = getConfiguredMacs;
            _isHome = ComputeIsHome();

            // Network changes (dock/undock, Wi-Fi roam, VPN up/down) trigger a debounced
            // re-evaluation; ARP can fail right after a change, so the periodic poll is the
            // safety net that converges within a minute.
            NetworkChange.NetworkAddressChanged += OnNetworkChanged;
            NetworkChange.NetworkAvailabilityChanged += OnNetworkAvailabilityChanged;
            _debounceTimer = new Timer(_ => Evaluate(), null, Timeout.Infinite, Timeout.Infinite);
            _pollTimer = new Timer(_ => Evaluate(), null, TimeSpan.FromSeconds(60), TimeSpan.FromSeconds(60));

            Log.Information("HomeNetworkMonitor started. Enabled: {enabled}, initial state: {state}",
                IsEnabled, _isHome ? "home" : "away");
        }

        private void OnNetworkChanged(object? sender, EventArgs e) => ScheduleEvaluate();

        private void OnNetworkAvailabilityChanged(object? sender, NetworkAvailabilityEventArgs e) => ScheduleEvaluate();

        private void ScheduleEvaluate()
        {
            if (_disposed) return;
            _debounceTimer.Change(TimeSpan.FromSeconds(3), Timeout.InfiniteTimeSpan);
        }

        /// <summary>Re-evaluates the home state now and fires HomeStateChanged on a transition.</summary>
        public void Evaluate()
        {
            if (_disposed) return;

            bool home;
            try
            {
                home = ComputeIsHome();
            }
            catch (Exception ex)
            {
                Log.Error("HomeNetworkMonitor evaluation failed: {message}", ex.Message);
                return;
            }

            lock (_stateLock)
            {
                if (_isHome == home) return;
                _isHome = home;
            }

            Log.Information("Home network state changed: {state}", home ? "home" : "away");
            try
            {
                HomeStateChanged?.Invoke(home);
            }
            catch (Exception ex)
            {
                Log.Error(ex, "Error in HomeStateChanged handler");
            }
        }

        private bool ComputeIsHome()
        {
            var configured = ParseMacs(_getConfiguredMacs());
            if (configured.Count == 0)
                return true; // Feature disabled — behave like upstream.

            foreach (var gateway in GetIpv4Gateways())
            {
                var mac = TryResolveMac(gateway);
                if (mac != null && configured.Contains(mac))
                    return true;
            }
            return false;
        }

        /// <summary>
        /// Returns the MAC of the current default gateway formatted as AA:BB:CC:DD:EE:FF,
        /// or null when it cannot be determined. Used by the "use current network" button.
        /// </summary>
        public static string? GetCurrentGatewayMac()
        {
            foreach (var gateway in GetIpv4Gateways())
            {
                var mac = TryResolveMac(gateway);
                if (mac != null)
                    return FormatMac(mac);
            }
            return null;
        }

        private static IEnumerable<IPAddress> GetIpv4Gateways()
        {
            var gateways = new List<IPAddress>();
            try
            {
                foreach (var nic in NetworkInterface.GetAllNetworkInterfaces())
                {
                    if (nic.OperationalStatus != OperationalStatus.Up)
                        continue;
                    if (nic.NetworkInterfaceType == NetworkInterfaceType.Loopback ||
                        nic.NetworkInterfaceType == NetworkInterfaceType.Tunnel)
                        continue;

                    foreach (var gw in nic.GetIPProperties().GatewayAddresses)
                    {
                        var address = gw.Address;
                        if (address.AddressFamily == AddressFamily.InterNetwork &&
                            !IPAddress.Any.Equals(address) &&
                            !gateways.Contains(address))
                        {
                            gateways.Add(address);
                        }
                    }
                }
            }
            catch (Exception ex)
            {
                Log.Error("Failed to enumerate network gateways: {message}", ex.Message);
            }
            return gateways;
        }

        /// <summary>Resolves a gateway's MAC via ARP, normalized to bare uppercase hex (no separators).</summary>
        private static string? TryResolveMac(IPAddress gateway)
        {
            try
            {
                uint destIp = BitConverter.ToUInt32(gateway.GetAddressBytes(), 0);
                var mac = new byte[6];
                uint macLen = (uint)mac.Length;
                int result = SendARP(destIp, 0, mac, ref macLen);
                if (result != 0 || macLen != 6)
                {
                    Log.Debug("SendARP for gateway {gateway} failed with code {code}", gateway, result);
                    return null;
                }
                return Convert.ToHexString(mac);
            }
            catch (Exception ex)
            {
                Log.Debug("ARP resolution for gateway {gateway} threw: {message}", gateway, ex.Message);
                return null;
            }
        }

        /// <summary>Parses a comma/semicolon/space separated MAC list into normalized bare hex strings.</summary>
        private static HashSet<string> ParseMacs(string? configured)
        {
            var macs = new HashSet<string>(StringComparer.OrdinalIgnoreCase);
            if (string.IsNullOrWhiteSpace(configured))
                return macs;

            foreach (var part in configured.Split(new[] { ',', ';', ' ' }, StringSplitOptions.RemoveEmptyEntries))
            {
                var normalized = part.Replace(":", "").Replace("-", "").Trim().ToUpperInvariant();
                if (normalized.Length == 12 && normalized.All(Uri.IsHexDigit))
                    macs.Add(normalized);
                else
                    Log.Warning("Ignoring invalid home gateway MAC entry: {entry}", part);
            }
            return macs;
        }

        private static string FormatMac(string bareHex) =>
            string.Join(":", Enumerable.Range(0, 6).Select(i => bareHex.Substring(i * 2, 2)));

        public void Dispose()
        {
            if (_disposed) return;
            _disposed = true;
            NetworkChange.NetworkAddressChanged -= OnNetworkChanged;
            NetworkChange.NetworkAvailabilityChanged -= OnNetworkAvailabilityChanged;
            _debounceTimer.Dispose();
            _pollTimer.Dispose();
        }
    }
}
