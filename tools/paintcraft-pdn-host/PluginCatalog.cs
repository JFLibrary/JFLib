using System.Diagnostics;
using System.IO;
using System.Reflection;
using System.Reflection.Metadata;
using System.Reflection.PortableExecutable;
using System.Runtime.Loader;
using System.Text;
using Microsoft.Win32;
using PaintDotNet;
using PaintDotNet.Effects;

// Each assembly is inspected in a fresh process. One broken dependency, static
// initializer, crash or hang cannot prevent other assemblies from being listed.
internal static class PluginCatalog
{
    internal static string? LoadedFrom;
    internal static Assembly Load(string path)
    {
        path=Path.GetFullPath(path);
        LoadedFrom=path;
        var dir=Path.GetDirectoryName(path)!;
        Environment.SetEnvironmentVariable("PATH",dir+Path.PathSeparator+Environment.GetEnvironmentVariable("PATH"));
        AssemblyLoadContext.Default.Resolving+=(c,n)=>{
            var p=Path.Combine(dir,n.Name+".dll");
            return File.Exists(p)?c.LoadFromAssemblyPath(p):null;
        };
        return new PluginContext(dir).LoadFromAssemblyPath(path);
    }
    // A plugin's own copy of a library wins: paint.net's ComputeSharp, for one, is trimmed to what
    // paint.net uses and lacks what plugin shaders need. paint.net and the framework stay shared,
    // because their types cross the boundary.
    sealed class PluginContext(string dir):AssemblyLoadContext(Path.GetFileName(dir))
    {
        static readonly string[] Shared=["PaintDotNet","System","Microsoft","Presentation","WindowsBase","Accessibility"];
        protected override Assembly? Load(AssemblyName name)
        {
            if(Shared.Any(s=>name.Name!.StartsWith(s,StringComparison.OrdinalIgnoreCase)))return null;
            var path=Path.Combine(dir,name.Name+".dll");
            return File.Exists(path) ? LoadFromAssemblyPath(path) : null;
        }
    }
    // All types, not only exported ones: effect classes of the v5 samples are internal. A plugin
    // with an unloadable optional helper type still works, so load errors only count without effects.
    static Type[] Types(Assembly assembly,out IEnumerable<string> errors)
    {
        errors=[];
        try {return assembly.GetTypes();}
        catch(ReflectionTypeLoadException e) {
            errors=e.LoaderExceptions.Where(x=>x!=null).Select(x=>x!.Message).Distinct();
            return e.Types.Where(t=>t!=null).Cast<Type>().ToArray();
        }
    }
    internal static bool CanCreate(Type t)=>!t.IsAbstract && !t.ContainsGenericParameters && t.GetConstructor(BindingFlags.Instance|BindingFlags.Public|BindingFlags.NonPublic,Type.EmptyTypes)!=null;
    internal static void Row(string kind,string path,string type,string name,string flags,string extensions)=>Console.WriteLine(RowText(kind,path,type,name,flags,extensions));
    // Hex-encoded UTF-8 fields keep paths, Unicode and error text unambiguous.
    static string RowText(string kind,string path,string type,string name,string flags,string extensions)=>
        string.Join('\t',kind,Convert.ToHexString(Encoding.UTF8.GetBytes(path)),
            Convert.ToHexString(Encoding.UTF8.GetBytes(type)),Convert.ToHexString(Encoding.UTF8.GetBytes(name)),
            flags,Convert.ToHexString(Encoding.UTF8.GetBytes(extensions)));
    internal static int Inspect(string path)
    {
        using(var stream=File.OpenRead(path)) using(var pe=new PEReader(stream)) if(!pe.HasMetadata)return 0;
        bool found=false;
        var all=Types(Load(path),out var errors);
        foreach(var t in all.Where(CanCreate)) {
            try {
                if(typeof(Effect).IsAssignableFrom(t)) {
                    using var effect=(Effect)Activator.CreateInstance(t,true)!;
                    Row("E",path,t.FullName!,effect.Name,"0","");found=true;
                } else if(typeof(EffectBase).IsAssignableFrom(t)) {
                    using var effect=(IEffect)Activator.CreateInstance(t,true)!;
                    // Flags: 1 modern CPU, 2 GPU. Both run through the same renderer.
                    if(effect.IsRendererAvailable<IBitmapEffectRenderer>())Row("E",path,t.FullName!,effect.EffectInfo.Name,typeof(PaintDotNet.Effects.Gpu.GpuEffect).IsAssignableFrom(t)?"2":"1","");
                    else Row("U",path,t.FullName!,t.Name+": Dieser Effekt bietet keinen Bitmap-Renderer an.","0","");
                    found=true;
                } else if(typeof(IFileTypeFactory).IsAssignableFrom(t)) {
                    var types=((IFileTypeFactory)Activator.CreateInstance(t,true)!).GetFileTypeInstances();
                    for(int i=0;i<types.Length;i++) FileRow(path,t.FullName+"#"+i,types[i]);
                    found=true;
                } else if(typeof(IFileTypeFactory2).IsAssignableFrom(t)) {
                    var types=((IFileTypeFactory2)Activator.CreateInstance(t,true)!).GetFileTypeInstances(new FileTypeHost());
                    for(int i=0;i<types.Length;i++) FileRow(path,t.FullName+"#"+i,types[i]);
                    found=true;
                } else if(typeof(FileType).IsAssignableFrom(t)) {
                    FileRow(path,t.FullName!, (FileType)Activator.CreateInstance(t,true)!);found=true;
                }
            } catch(Exception e) {Row("X",path,t.FullName!,e.GetBaseException().Message,"0","");found=true;}
        }
        if(!found)foreach(var error in errors)Row("X",path,"",error,"0","");
        return 0;
    }
    static void FileRow(string path,string type,FileType ft)
    {
        var options=ft.Options;
        var exts=new Dictionary<string,int>(StringComparer.OrdinalIgnoreCase);
        void Add(IEnumerable<string> extensions,int cap) {
            foreach(var e in extensions.Select(e=>e.TrimStart('.')).Where(e=>e.Length is >0 and <=29 && e.All(c=>char.IsAsciiLetterOrDigit(c)||c=='_'||c=='-'))) {
                exts.TryGetValue(e,out int flags);exts[e]=flags|cap;
            }
        }
        if(options.SupportsLoading)Add(options.LoadExtensions,1);
        if(options.SupportsSaving)Add(options.SaveExtensions,2);
        foreach(var group in exts.GroupBy(x=>x.Value))Row("F",path,type,ft.Name,(group.Key|(options.SupportsLayers?4:0)).ToString(),string.Join(';',group.Select(x=>x.Key)));
    }
    internal static int Scan(string[] args,string install)
    {
        var roots=new List<string>();
        for(int i=1;i+1<args.Length;i++) if(args[i]=="--root")roots.Add(args[++i]);
        if(!args.Contains("--isolated")) {
            var docs=Environment.GetFolderPath(Environment.SpecialFolder.MyDocuments);
            roots.Add(Path.Combine(docs,"PaintCraft App Files"));
            roots.Add(Path.Combine(docs,"Paint.NET App Files"));
            roots.Add(install);
            using var machine=Registry.LocalMachine.OpenSubKey(@"Software\Paint.NET");
            if(string.Equals(machine?.GetValue("Plugins/AllowAdditionalPluginDirectoryRoots")?.ToString(),"true",StringComparison.OrdinalIgnoreCase)) {
                using var user=Registry.CurrentUser.OpenSubKey(@"Software\Paint.NET");
                roots.AddRange((user?.GetValue("Plugins/AdditionalPluginDirectoryRoots")?.ToString()??"").Split(';',StringSplitOptions.RemoveEmptyEntries).Select(Environment.ExpandEnvironmentVariables));
            }
        }
        var seen=new HashSet<string>(StringComparer.OrdinalIgnoreCase);
        var plugins=new List<string>();
        foreach(var root in roots.Distinct(StringComparer.OrdinalIgnoreCase)) foreach(var folder in new[]{"Effects","FileTypes","Shapes"}) {
            var directory=Path.Combine(root,folder);
            if(!Directory.Exists(directory))continue;
            string[] files;
            // Plugins with dependencies live in their own subfolder (paint.net 5's layout).
            var options=new EnumerationOptions {RecurseSubdirectories=folder!="Shapes",MaxRecursionDepth=4,IgnoreInaccessible=true,
                AttributesToSkip=FileAttributes.Hidden|FileAttributes.System|FileAttributes.ReparsePoint};
            try {files=Directory.GetFiles(directory,folder=="Shapes"?"*.xaml":"*.dll",options).Order(StringComparer.OrdinalIgnoreCase).ToArray();}
            catch(Exception e) {Row("X",directory,"",e.Message,"0","");continue;}
            foreach(var file in files) {
                var full=Path.GetFullPath(file);
                if(!seen.Add(full))continue;
                if(folder=="Shapes") {Row("S",full,"",Path.GetFileNameWithoutExtension(full),"0","");continue;}
                try {if(!IsPlugin(full))continue;}
                catch(Exception e) {Row("X",full,"",e.GetBaseException().Message,"0","");continue;}
                if(plugins.Count==256){Row("X",full,"","Maximal 256 Plugin-DLLs pro Start","0","");break;}
                plugins.Add(full);
            }
        }
        // A process per DLL, a few at a time; the rows keep the order of the files.
        var rows=new string[plugins.Count];
        Parallel.For(0,plugins.Count,new ParallelOptions {MaxDegreeOfParallelism=Math.Clamp(Environment.ProcessorCount,1,4)},
            i=>rows[i]=InspectInChild(args,install,plugins[i]));
        foreach(var row in rows)Console.Write(row);
        return 0;
    }
    // Dependencies share a plugin's folder; only assemblies built against paint.net can hold plugins.
    static bool IsPlugin(string path)
    {
        using var stream=File.OpenRead(path);
        using var pe=new PEReader(stream);
        if(!pe.HasMetadata)return false;
        var metadata=pe.GetMetadataReader();
        return metadata.AssemblyReferences.Any(r=>metadata.GetString(metadata.GetAssemblyReference(r).Name).StartsWith("PaintDotNet",StringComparison.OrdinalIgnoreCase));
    }
    static string InspectInChild(string[] args,string install,string full)
    {
        var failure=(string message)=>RowText("X",full,"",message,"0","")+"\n";
        try {
            var executable=Environment.ProcessPath ?? "dotnet";
            var name=Path.GetFileNameWithoutExtension(executable);
            bool dotnet=name.Equals("dotnet",StringComparison.OrdinalIgnoreCase);
            var start=new ProcessStartInfo(executable) {UseShellExecute=false,CreateNoWindow=true,RedirectStandardOutput=true,RedirectStandardError=true,StandardErrorEncoding=Encoding.UTF8};
            if(dotnet) start.ArgumentList.Add(Assembly.GetExecutingAssembly().Location);
            else if(!name.Equals("PaintCraft.PdnHost",StringComparison.OrdinalIgnoreCase)) {
                // PaintCraft started this helper on paint.net's own runtime and does so again.
                start.ArgumentList.Add("--pdn-clr");start.ArgumentList.Add(install);start.ArgumentList.Add(Assembly.GetExecutingAssembly().Location);
            }
            start.ArgumentList.Add(full);start.ArgumentList.Add("--inspect");
            HostWatchdog.Forward(args,start);
            // The installed .NET can differ from the build paint.net precompiled its images for.
            if(dotnet) start.Environment["DOTNET_ReadyToRun"]="0";
            using var child=Process.Start(start)!;
            var stdout=ReadLimited(child.StandardOutput);var stderr=ReadLimited(child.StandardError);
            if(!child.WaitForExit(15000)){child.Kill(true);child.WaitForExit();return failure("Zeitlimit beim Laden (15 Sekunden)");}
            return child.ExitCode==0 ? stdout.GetAwaiter().GetResult() : failure(stderr.GetAwaiter().GetResult());
        } catch(Exception e) {return failure(e.GetBaseException().Message);}
    }
    static async Task<string> ReadLimited(StreamReader reader)
    {
        var text=new StringBuilder();var buffer=new char[4096];int n;
        while((n=await reader.ReadAsync(buffer))!=0) {if(text.Length<1_000_000)text.Append(buffer,0,Math.Min(n,1_000_000-text.Length));}
        return text.ToString();
    }
}
