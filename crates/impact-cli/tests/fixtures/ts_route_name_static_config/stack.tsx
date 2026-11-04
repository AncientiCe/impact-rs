import { HomeScreen } from './home'
import { DetailsScreen } from './details'

export const AppStack = createStackNavigator({
  screens: {
    Home: { screen: HomeScreen, options: () => ({}) },
    Details: DetailsScreen,
  },
})
