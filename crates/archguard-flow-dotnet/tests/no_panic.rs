//! The .NET readers take foreign source: broken input must never panic.
#[path = "../../archguard-flow-common/tests/support/dirs.rs"]
mod dirs;
#[path = "../../archguard-flow-common/tests/support/variants.rs"]
mod variants;

use archguard_flow_dotnet::{domain_csharp, ui_flow_blazor, ui_flow_xaml};

const CS: &str = r#"
namespace A.B;
using Microsoft.AspNetCore.Components;
[Authorize(Roles = "admin")]
public class Order { public int Id { get; set; } public List<Line> Lines { get; set; } }
public class Db : DbContext { public DbSet<Order> Orders { get; set; } }
public class Vm : ObservableObject { [RelayCommand] void Save() { nav.NavigateTo("/x"); } }
"#;
const RAZOR: &str = r#"@page "/orders/{Id:int}"
@attribute [Authorize(Roles = "admin")]
@inject NavigationManager Nav
<EditForm Model="m" OnValidSubmit="Save"><InputText @bind-Value="m.Name" /></EditForm>
<NavLink href="/other">go</NavLink>
@code { void Save() { Nav.NavigateTo("/done"); } }
"#;
const XAML: &str = r#"<Window x:Class="A.Main" xmlns="http://schemas.microsoft.com/winfx/2006/xaml/presentation">
  <Grid><Button Content="Go" Command="{Binding SaveCommand}" Click="OnClick"/>
  <Frame Source="Other.xaml"/></Grid></Window>"#;

#[test]
fn dotnet_extractors_survive_broken_files() {
    let files = [
        (
            "App.csproj",
            "<Project Sdk=\"Microsoft.NET.Sdk.Web\"><PropertyGroup><TargetFramework>net8.0</TargetFramework></PropertyGroup></Project>",
        ),
        ("Models/Order.cs", CS),
        ("Pages/Orders.razor", RAZOR),
        ("Views/Main.xaml", XAML),
        ("Views/Main.xaml.cs", CS),
    ];
    dirs::each_broken_tree(&files, |root| {
        domain_csharp::extract(root);
        ui_flow_blazor::extract(root);
        ui_flow_xaml::extract(root);
    });
}
